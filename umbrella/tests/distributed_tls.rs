//! D17 follow-up: the linear wrappers run unchanged while every host-authority
//! verb travels over a live mutual-TLS connection to a separate process.
//!
//! The parent process compiles and JIT-executes `DistributedRequest.aum`; the
//! child process owns the `Authority` and answers the 18 `au_dist_*` verbs on a
//! loopback TLS socket. The module's own `au_dist_respond` call therefore also
//! crosses the transport instead of reaching an in-process authority.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use austral_cranelift_bridge::distributed::{
    apply, au_dist_adopt_wire, au_dist_authorize, au_dist_introduce, au_dist_issue_token,
    au_dist_open_request, au_dist_receipt_code, au_dist_receipt_release, au_dist_release_resource,
    au_dist_reset, au_dist_respond, au_dist_retire, au_dist_retry, au_dist_submit,
    au_dist_token_wire, au_dist_wait, set_transport, Authority, Operation, Transport,
    RESULT_APPLIED, STATUS_ALREADY_SENT, STATUS_CAPACITY, STATUS_DUPLICATE, STATUS_NO_REQUEST,
    STATUS_NO_TOKEN, STATUS_RETIRED_EPOCH, STATUS_STALE_RESOURCE,
};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, SanType,
};
use rustls::pki_types::{
    CertificateDer, IpAddr as TlsIpAddr, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName,
};
use rustls::server::WebPkiClientVerifier;
use rustls::{
    ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection, StreamOwned,
};

const FRAME: usize = 1 + 5 * 8;

fn provider() {
    // `install_default` fails only if another test already installed one.
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn key_der(key: &KeyPair) -> PrivateKeyDer<'static> {
    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()))
}

fn encode(operation: Operation) -> [u8; FRAME] {
    let (tag, fields): (u8, [i64; 5]) = match operation {
        Operation::Reset => (0, [0; 5]),
        Operation::Introduce {
            resource,
            deployment,
            owner,
            epoch,
        } => (1, [resource, deployment, owner, epoch, 0]),
        Operation::Retire {
            resource,
            new_epoch,
        } => (2, [resource, new_epoch, 0, 0, 0]),
        Operation::IssueToken {
            resource,
            owner,
            deployment,
            epoch,
        } => (3, [resource, owner, deployment, epoch, 0]),
        Operation::TokenWire { token } => (4, [token, 0, 0, 0, 0]),
        Operation::AdoptWire { wire } => (5, [wire, 0, 0, 0, 0]),
        Operation::OpenRequest {
            token,
            request_id,
            digest,
        } => (6, [token, request_id, digest, 0, 0]),
        Operation::Submit { request } => (7, [request, 0, 0, 0, 0]),
        Operation::Retry { request } => (8, [request, 0, 0, 0, 0]),
        Operation::Wait { request } => (9, [request, 0, 0, 0, 0]),
        Operation::CancelWait { request } => (10, [request, 0, 0, 0, 0]),
        Operation::ReleaseRequest { request } => (11, [request, 0, 0, 0, 0]),
        Operation::Respond {
            request_id,
            digest,
            accepted,
        } => (12, [request_id, digest, accepted, 0, 0]),
        Operation::ReceiptCode { receipt } => (13, [receipt, 0, 0, 0, 0]),
        Operation::ReceiptRelease { receipt } => (14, [receipt, 0, 0, 0, 0]),
        Operation::Authorize { token, resource } => (15, [token, resource, 0, 0, 0]),
        Operation::ReleaseToken { token } => (16, [token, 0, 0, 0, 0]),
        Operation::ReleaseResource { resource } => (17, [resource, 0, 0, 0, 0]),
    };
    let mut out = [0u8; FRAME];
    out[0] = tag;
    for (index, value) in fields.iter().enumerate() {
        let start = 1 + index * 8;
        out[start..start + 8].copy_from_slice(&value.to_le_bytes());
    }
    out
}

fn field(frame: &[u8; FRAME], index: usize) -> i64 {
    let start = 1 + index * 8;
    i64::from_le_bytes(frame[start..start + 8].try_into().expect("field width"))
}

fn decode(frame: &[u8; FRAME]) -> Option<Operation> {
    let values = [
        field(frame, 0),
        field(frame, 1),
        field(frame, 2),
        field(frame, 3),
        field(frame, 4),
    ];
    Some(match frame[0] {
        0 => Operation::Reset,
        1 => Operation::Introduce {
            resource: values[0],
            deployment: values[1],
            owner: values[2],
            epoch: values[3],
        },
        2 => Operation::Retire {
            resource: values[0],
            new_epoch: values[1],
        },
        3 => Operation::IssueToken {
            resource: values[0],
            owner: values[1],
            deployment: values[2],
            epoch: values[3],
        },
        4 => Operation::TokenWire { token: values[0] },
        5 => Operation::AdoptWire { wire: values[0] },
        6 => Operation::OpenRequest {
            token: values[0],
            request_id: values[1],
            digest: values[2],
        },
        7 => Operation::Submit { request: values[0] },
        8 => Operation::Retry { request: values[0] },
        9 => Operation::Wait { request: values[0] },
        10 => Operation::CancelWait { request: values[0] },
        11 => Operation::ReleaseRequest { request: values[0] },
        12 => Operation::Respond {
            request_id: values[0],
            digest: values[1],
            accepted: values[2],
        },
        13 => Operation::ReceiptCode { receipt: values[0] },
        14 => Operation::ReceiptRelease { receipt: values[0] },
        15 => Operation::Authorize {
            token: values[0],
            resource: values[1],
        },
        16 => Operation::ReleaseToken { token: values[0] },
        17 => Operation::ReleaseResource {
            resource: values[0],
        },
        _ => return None,
    })
}

struct TlsTransport {
    stream: StreamOwned<ClientConnection, TcpStream>,
}

impl Drop for TlsTransport {
    fn drop(&mut self) {
        // A clean TLS shutdown is part of the harness: the authority treats an
        // unexpected EOF as a transport error, not as a completed session.
        self.stream.conn.send_close_notify();
        let _ = self.stream.flush();
    }
}

impl Transport for TlsTransport {
    fn call(&mut self, operation: Operation) -> i64 {
        self.stream
            .write_all(&encode(operation))
            .expect("write D17 transport frame");
        let mut response = [0u8; 8];
        self.stream
            .read_exact(&mut response)
            .expect("read D17 transport response");
        i64::from_le_bytes(response)
    }
}

fn read_cert(path: &Path) -> CertificateDer<'static> {
    CertificateDer::from(std::fs::read(path).expect("read certificate"))
}

fn read_key(path: &Path) -> PrivateKeyDer<'static> {
    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        std::fs::read(path).expect("read private key"),
    ))
}

fn certificate(
    common_name: &str,
    san: Option<IpAddr>,
    usage: ExtendedKeyUsagePurpose,
    ca: &rcgen::Certificate,
    ca_key: &KeyPair,
) -> (rcgen::Certificate, KeyPair) {
    let key = KeyPair::generate().expect("key");
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("params");
    params
        .distinguished_name
        .push(DnType::CommonName, common_name);
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![usage];
    if let Some(address) = san {
        params.subject_alt_names = vec![SanType::IpAddress(address)];
    }
    let cert = params.signed_by(&key, ca, ca_key).expect("signed cert");
    (cert, key)
}

fn write_identity(dir: &Path, name: &str, cert: &rcgen::Certificate, key: &KeyPair) {
    std::fs::write(dir.join(format!("{name}.der")), cert.der().as_ref()).expect("write cert");
    std::fs::write(dir.join(format!("{name}.key")), key.serialize_der()).expect("write key");
}

fn generate_identities(
    dir: &Path,
) -> (
    CertificateDer<'static>,
    CertificateDer<'static>,
    PrivateKeyDer<'static>,
) {
    let ca_key = KeyPair::generate().expect("CA key");
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("CA params");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "D17 TLS test CA");
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca = ca_params.self_signed(&ca_key).expect("CA cert");

    let (server_cert, server_key) = certificate(
        "D17 TLS authority",
        Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        ExtendedKeyUsagePurpose::ServerAuth,
        &ca,
        &ca_key,
    );
    let (client_cert, client_key) = certificate(
        "D17 TLS requester",
        None,
        ExtendedKeyUsagePurpose::ClientAuth,
        &ca,
        &ca_key,
    );

    std::fs::write(dir.join("ca.der"), ca.der().as_ref()).expect("write CA");
    write_identity(dir, "server", &server_cert, &server_key);
    write_identity(dir, "client", &client_cert, &client_key);

    (
        ca.der().clone(),
        client_cert.der().clone(),
        key_der(&client_key),
    )
}

#[test]
fn d17_tls_server_child() {
    if std::env::var_os("D17_TLS_SERVER").is_none() {
        return;
    }
    provider();
    let dir = PathBuf::from(std::env::var("D17_TLS_DIR").expect("server directory"));
    let mut roots = RootCertStore::empty();
    roots
        .add(read_cert(&dir.join("ca.der")))
        .expect("trust test CA");
    let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .expect("client verifier");
    let config = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            vec![read_cert(&dir.join("server.der"))],
            read_key(&dir.join("server.key")),
        )
        .expect("server TLS config");

    let listener = TcpListener::bind((IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).expect("bind");
    let port = listener.local_addr().expect("local address").port();
    std::fs::write(dir.join("ready"), port.to_string()).expect("signal readiness");
    let (socket, _) = listener.accept().expect("accept requester");
    let connection = ServerConnection::new(Arc::new(config)).expect("server connection");
    let mut stream = StreamOwned::new(connection, socket);

    let mut authority = match std::env::var("D17_TLS_REQUEST_ID_LIMIT") {
        Ok(limit) => {
            Authority::with_request_id_limit(limit.parse().expect("numeric request limit"))
        }
        Err(_) => Authority::default(),
    };
    loop {
        let mut frame = [0u8; FRAME];
        let mut filled = 0;
        while filled < FRAME {
            match stream.read(&mut frame[filled..]) {
                Ok(0) => return,
                Ok(count) => filled += count,
                Err(error) => panic!("read D17 transport frame: {error}"),
            }
        }
        let operation = decode(&frame).expect("known D17 operation");
        let status = apply(&mut authority, operation);
        stream
            .write_all(&status.to_le_bytes())
            .expect("write D17 transport response");
    }
}

fn wait_for_child(child: &mut std::process::Child) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait().expect("wait for server") {
            Some(status) => {
                assert!(status.success(), "D17 TLS server exited with {status}");
                return;
            }
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("D17 TLS server did not exit after the connection closed");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

#[test]
fn the_d17_wrappers_run_over_a_live_two_process_mutual_tls_transport() {
    provider();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().expect("umbrella is inside australVM");
    let pair = PathBuf::from(format!(
        "{}/stdlib/net/Distributed.aui,{}/stdlib/net/Distributed.aum",
        root.display(),
        root.display()
    ));
    let module = manifest.join("modules/DistributedRequest.aum");
    let austral = root.join("austral");

    let dependency = umbrella::frontend::compile(&austral, &[pair.clone()], false)
        .expect("the umbrella front-end must emit the Distributed module's CPS");
    let artifact = umbrella::frontend::compile(&austral, &[pair, module], false)
        .expect("the umbrella front-end must emit the D17 module's CPS");

    let dir = std::env::temp_dir().join(format!("d17-tls-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create harness directory");
    let (ca, client_cert, client_key) = generate_identities(&dir);

    let mut child = Command::new(std::env::current_exe().expect("test executable"))
        .arg("--exact")
        .arg("d17_tls_server_child")
        .arg("--nocapture")
        .env("D17_TLS_SERVER", "1")
        .env("D17_TLS_DIR", &dir)
        .env("D17_TLS_REQUEST_ID_LIMIT", "2")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn D17 TLS authority process");

    let ready = dir.join("ready");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        if let Some(status) = child.try_wait().expect("probe server") {
            let _ = std::fs::remove_dir_all(&dir);
            panic!("D17 TLS server exited before listening: {status}");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = std::fs::remove_dir_all(&dir);
            panic!("D17 TLS server did not become ready");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let port: u16 = std::fs::read_to_string(&ready)
        .expect("read server port")
        .trim()
        .parse()
        .expect("numeric server port");

    let mut roots = RootCertStore::empty();
    roots.add(ca).expect("trust test CA");
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(vec![client_cert], client_key)
        .expect("client TLS config");
    let connection = ClientConnection::new(
        Arc::new(config),
        ServerName::IpAddress(TlsIpAddr::V4(Ipv4Addr::LOCALHOST.into())),
    )
    .expect("client connection");
    let socket = TcpStream::connect((IpAddr::V4(Ipv4Addr::LOCALHOST), port)).expect("connect");
    set_transport(Some(Box::new(TlsTransport {
        stream: StreamOwned::new(connection, socket),
    })));

    // The reset crosses the same transport; the child's authority is fresh, so
    // this is both a liveness probe and a deterministic starting point.
    assert_eq!(au_dist_reset(), 0);

    austral_cranelift_bridge::auth::set_allow_all();
    let dependency_fn = austral_cranelift_bridge::compile_to_function_named(
        dependency.cps.as_ptr(),
        dependency.cps.len(),
        std::ptr::null(),
        0,
    );
    assert!(
        !dependency_fn.is_null(),
        "the bridge must compile the Distributed definitions"
    );
    let entry = b"run";
    let function = austral_cranelift_bridge::compile_entry_i64(
        artifact.cps.as_ptr(),
        artifact.cps.len(),
        entry.as_ptr(),
        entry.len(),
        0,
    );
    if function.is_null() {
        set_transport(None);
        let _ = child.kill();
        let _ = std::fs::remove_dir_all(&dir);
        panic!("the bridge must compile the D17 entry");
    }
    let result = austral_cranelift_bridge::execute_function(function);

    // Replay refusals must survive the process boundary too. The module above
    // has already released resource 101, so these calls start from a clean
    // authority and use the same transport as every module verb.
    assert_eq!(au_dist_introduce(101, 202, 303, 1), 101);
    let token = au_dist_issue_token(101, 303, 202, 1);
    let wire = au_dist_token_wire(token);
    assert!(wire > 0);
    assert_eq!(au_dist_release_resource(101), 0);
    assert_eq!(au_dist_introduce(101, 202, 303, 1), 101);
    assert_eq!(au_dist_adopt_wire(wire), STATUS_STALE_RESOURCE);
    assert_eq!(au_dist_adopt_wire(wire), STATUS_STALE_RESOURCE);

    let token = au_dist_issue_token(101, 303, 202, 1);
    let request = au_dist_open_request(token, 7, 11);
    assert!(request > 0);
    let duplicate = au_dist_issue_token(101, 303, 202, 1);
    assert_eq!(au_dist_open_request(duplicate, 7, 11), STATUS_DUPLICATE);
    assert_eq!(au_dist_submit(request), 0);
    assert_eq!(au_dist_respond(7, 11, 1), 0);
    assert_eq!(au_dist_respond(7, 11, 1), STATUS_ALREADY_SENT);
    let receipt = au_dist_wait(request);
    assert!(receipt > 0);
    assert_eq!(au_dist_receipt_code(receipt), RESULT_APPLIED);
    assert_eq!(au_dist_respond(7, 11, 1), STATUS_NO_REQUEST);
    assert_eq!(au_dist_receipt_release(receipt), 0);
    assert_eq!(au_dist_release_resource(101), 0);

    assert_eq!(au_dist_introduce(103, 202, 303, 1), 103);
    let token = au_dist_issue_token(103, 303, 202, 1);
    let retired_wire = au_dist_token_wire(token);
    assert!(retired_wire > 0);
    let token = au_dist_issue_token(103, 303, 202, 1);
    let pending = au_dist_open_request(token, 9, 13);
    assert_eq!(au_dist_submit(pending), 0);
    assert_eq!(au_dist_retire(103, 2), 0);
    assert_eq!(au_dist_retry(pending), STATUS_NO_REQUEST);
    assert_eq!(au_dist_respond(9, 13, 1), STATUS_NO_REQUEST);
    assert_eq!(au_dist_adopt_wire(retired_wire), STATUS_NO_TOKEN);
    assert_eq!(au_dist_issue_token(103, 303, 202, 1), STATUS_RETIRED_EPOCH);
    let fresh = au_dist_issue_token(103, 303, 202, 2);
    assert!(fresh > 0);
    assert_eq!(au_dist_authorize(fresh, 103), 0);
    assert_eq!(au_dist_release_resource(103), 0);

    // The child's request-id cap is lowered to 2 for the harness, so the same
    // capacity refusal is observable over TLS without 65k round trips.
    assert_eq!(au_dist_introduce(104, 202, 303, 1), 104);
    for id in 1..=2 {
        let token = au_dist_issue_token(104, 303, 202, 1);
        assert!(au_dist_open_request(token, id, 11) > 0);
    }
    let token = au_dist_issue_token(104, 303, 202, 1);
    assert_eq!(au_dist_open_request(token, 3, 11), STATUS_CAPACITY);
    assert_eq!(au_dist_release_resource(104), 0);
    assert_eq!(au_dist_introduce(104, 202, 303, 1), 104);
    let token = au_dist_issue_token(104, 303, 202, 1);
    assert!(
        au_dist_open_request(token, 3, 11) > 0,
        "release frees the cap"
    );
    assert_eq!(au_dist_release_resource(104), 0);

    set_transport(None);
    wait_for_child(&mut child);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        result, 0,
        "the D17 request must finish applied at the remote authority"
    );
}
