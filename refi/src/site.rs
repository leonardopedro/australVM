//! Server-rendered pages. The public site and the admin console are plain
//! functions from state to HTML, served by the same HTTP front as the API.
//!
//! Every interpolated value goes through `escape_html`; page bodies go through
//! `content::render_markdown`, which escapes before applying its own small tag
//! set. There is no template engine and no client-side script, so the content
//! security policy can stay at `default-src 'none'`.

use serde_json::Value;
use unfer_taler::WireStatus;

use crate::accounts::Account;
use crate::content::{Page, render_markdown};
use crate::proofs::{Proof, ProofDetail};
use crate::util::escape_html;

/// Everything the chrome (nav/footer) needs.
pub struct Chrome<'a> {
    pub pages: &'a [(String, String)],
    pub sandbox: bool,
    pub base_url: &'a str,
}

fn layout(title: &str, chrome: &Chrome, body: &str, admin: bool) -> String {
    let nav_pages: String = chrome
        .pages
        .iter()
        .map(|(slug, title)| {
            format!(
                "<a href=\"/pages/{}\">{}</a>",
                escape_html(slug),
                escape_html(title)
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let admin_link = if admin {
        "<a class=\"admin-link\" href=\"/admin\">admin</a>"
    } else {
        ""
    };
    let banner = if chrome.sandbox {
        "<div class=\"banner\"><strong>Sandbox settlement.</strong> Bank wires are simulated \
         in-process; no fiat, securities or carbon instruments change hands.</div>"
    } else {
        ""
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{title} · ReFi Exchange</title>\
         <link rel=\"stylesheet\" href=\"/style.css\">\
         <meta name=\"description\" content=\"GNU Taler-style settlement over the unfer carbon-certificate ledger\">\
         </head><body>{banner}\
         <header class=\"top\"><a class=\"brand\" href=\"/\"><span class=\"mark\">◈</span> ReFi Exchange</a>\
         <nav><a href=\"/\">Home</a><a href=\"/registry\">Registry</a><a href=\"/proofs\">Proofs</a><a href=\"/api\">API</a>{nav_pages}{admin_link}</nav>\
         </header><main>{body}</main>\
         <footer><span>GNU Taler-style exchange adapter · unfer certificate ledger</span>\
         <span class=\"mono\">{base}</span>\
         <span><a href=\"/pages/sandbox\">sandbox notice</a></span></footer>\
         </body></html>",
        title = escape_html(title),
        banner = banner,
        nav_pages = nav_pages,
        admin_link = admin_link,
        body = body,
        base = escape_html(chrome.base_url),
    )
}

fn stat(label: &str, value: &str, hint: &str) -> String {
    format!(
        "<div class=\"stat\"><div class=\"stat-label\">{}</div><div class=\"stat-value\">{}</div><div class=\"stat-hint\">{}</div></div>",
        escape_html(label),
        escape_html(value),
        escape_html(hint)
    )
}

fn short(s: &str) -> String {
    if s.len() > 18 {
        format!("{}…{}", &s[..10], &s[s.len() - 6..])
    } else {
        s.to_string()
    }
}

/// The landing page: live ledger numbers, the newest certificates and the
/// published content pages.
pub fn home(chrome: &Chrome, status: &Value, registry: &Value) -> String {
    let ledger = &status["ledger"];
    let supply = ledger["total_supply"].as_u64().unwrap_or(0);
    let unspent = ledger["unspent"].as_u64().unwrap_or(0);
    let root = ledger["root"].as_str().unwrap_or("");
    let reserves = status["reserves_total"].as_u64().unwrap_or(0);
    let merchants = status["merchants_total"].as_u64().unwrap_or(0);
    let seq = status["seq"].as_u64().unwrap_or(0);
    let treasury = status["treasury_did"].as_str().unwrap_or("");

    let rows = registry["certificates"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .take(8)
        .map(|c| {
            format!(
                "<tr><td class=\"mono\">{}</td><td>{}</td><td class=\"num\">{}</td><td>{}</td><td class=\"mono small\">{}</td><td><span class=\"pill {}\">{}</span></td></tr>",
                escape_html(&short(c["cert_id"].as_str().unwrap_or(""))),
                escape_html(c["kind"].as_str().unwrap_or("certificate")),
                c["amount"].as_u64().unwrap_or(0),
                escape_html(&short(c["owner"].as_str().unwrap_or(""))),
                escape_html(c["source"].as_str().unwrap_or("—")),
                if c["status"] == "unspent" { "ok" } else { "muted" },
                escape_html(c["status"].as_str().unwrap_or(""))
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let table = if rows.is_empty() {
        "<p class=\"empty\">No certificates minted yet. The treasury mints against a UNFCCC provenance reference; see the <a href=\"/api\">API</a>.</p>".to_string()
    } else {
        format!(
            "<table><thead><tr><th>Certificate</th><th>Kind</th><th class=\"num\">Amount</th><th>Owner</th><th>Provenance</th><th>Status</th></tr></thead><tbody>{rows}</tbody></table>"
        )
    };

    let body = format!(
        "<section class=\"hero\"><div class=\"kicker\">Carbon certificates · Taler settlement</div>\
         <h1>Verified tonnes, settled as spendable value.</h1>\
         <p class=\"lede\">A certificate ledger that proves conservation and a GNU Taler-style exchange \
         that turns reserve credit into e-coins — the same signed, replayable state for both fiat-backed \
         value and UNFCCC-backed carbon.</p>\
         <div class=\"cta\"><a class=\"button\" href=\"/api\">Read the API</a>\
         <a class=\"button ghost\" href=\"/registry\">Open the registry</a></div></section>\
         <section class=\"stats\">{}{}{}{}</section>\
         <section class=\"panel\"><div class=\"panel-head\"><h2>Ledger state</h2>\
         <span class=\"mono small\">seq {seq} · root {root}</span></div>\
         <div class=\"kv\"><div><span>Mint authority</span><code>{treasury}</code></div>\
         <div><span>Fiat in / out</span><code>{fin} / {fout}</code></div>\
         <div><span>Reserve liabilities</span><code>{reserves}</code></div>\
         <div><span>Merchant balances</span><code>{merchants}</code></div>\
         <div><span>Outstanding e-coins</span><code>{outstanding}</code></div></div></section>\
         <section class=\"panel\"><div class=\"panel-head\"><h2>Newest certificates</h2>\
         <a href=\"/registry\" class=\"small\">full registry →</a></div>{table}</section>",
        stat("Certificates in circulation", &supply.to_string(), &format!("{unspent} unspent")),
        stat("Ledger sequence", &seq.to_string(), "signed ops"),
        stat("Reserve liabilities", &reserves.to_string(), "sandbox units"),
        stat("Merchant balances", &merchants.to_string(), "redeemable"),
        seq = seq,
        root = escape_html(&short(root)),
        treasury = escape_html(treasury),
        fin = status["fiat"]["in"].as_u64().unwrap_or(0),
        fout = status["fiat"]["out"].as_u64().unwrap_or(0),
        reserves = reserves,
        merchants = merchants,
        outstanding = status["funded_outstanding"].as_u64().unwrap_or(0),
        table = table,
    );
    layout("Home", chrome, &body, false)
}

/// The full certificate registry, newest first.
pub fn registry(
    chrome: &Chrome,
    registry: &Value,
    proofs: &[(String, String, String)],
) -> String {
    let certificates: Vec<Value> = registry["certificates"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    // The "backed by" column exists only when something is backed: a table
    // with a permanently empty column is noise for the common case.
    let attested = certificates
        .iter()
        .any(|c| c["kind"].as_str() == Some("attested"));

    let backed_by = |c: &Value| -> String {
        if c["kind"].as_str() != Some("attested") {
            return "<span class=\"muted\">—</span>".to_string();
        }
        let doi = c["source"].as_str().unwrap_or("");
        match proofs.iter().find(|(d, _, _)| d == doi) {
            Some((_, id, title)) => format!(
                "<a href=\"/proofs/{}\">{}</a>",
                escape_html(id),
                escape_html(title)
            ),
            None => "<span class=\"muted\">—</span>".to_string(),
        }
    };

    let header = if attested { "<th>Backed by</th>" } else { "" };
    let empty_colspan = if attested { 8 } else { 7 };
    let rows = certificates
        .into_iter()
        .map(|c| {
            let extra = if attested {
                format!("<td>{}</td>", backed_by(&c))
            } else {
                String::new()
            };
            format!(
                "<tr><td class=\"mono small\">{}</td><td>{}</td><td class=\"num\">{}</td><td class=\"mono small\">{}</td>\
                 <td class=\"mono small\">{}</td><td class=\"num\">{}</td><td><span class=\"pill {}\">{}</span></td>{extra}</tr>",
                escape_html(c["cert_id"].as_str().unwrap_or("")),
                escape_html(c["kind"].as_str().unwrap_or("certificate")),
                c["amount"].as_u64().unwrap_or(0),
                escape_html(c["owner"].as_str().unwrap_or("")),
                escape_html(c["source"].as_str().unwrap_or("\u{2014}")),
                c["seq"].as_u64().unwrap_or(0),
                if c["status"] == "unspent" { "ok" } else { "muted" },
                escape_html(c["status"].as_str().unwrap_or("")),
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let body = format!(
        "<section class=\"panel\"><div class=\"panel-head\"><h2>Certificate registry</h2>\
         <span class=\"small\">{}</span></div>\
         <p class=\"small muted\">Every mint on the consensus log, resolved against the live unspent set. \
         A spent row is history; an unspent row is a certificate that can still move.\
         An <code>attested</code> row was minted against a published proof's DOI, \
         <a href=\"/proofs\">backed by an archived verification</a>.</p>\
         <table><thead><tr><th>Certificate id</th><th>Kind</th><th class=\"num\">Amount</th><th>Owner</th>\
         <th>Provenance</th><th class=\"num\">Seq</th><th>Status</th>{header}</tr></thead>\
         <tbody>{}</tbody></table></section>",
        format!("{} rows", registry["count"].as_u64().unwrap_or(0)),
        if rows.is_empty() {
            format!("<tr><td colspan=\"{empty_colspan}\" class=\"empty\">Nothing minted yet.</td></tr>")
        } else {
            rows
        }
    );
    layout("Registry", chrome, &body, false)
}

/// A published content page.
pub fn page_view(chrome: &Chrome, page: &Page) -> String {
    let body = format!(
        "<article class=\"prose\"><div class=\"kicker\">Page · rev {}</div><h1>{}</h1>{}</article>\
         <p class=\"small muted\">Last edited {} by <span class=\"mono\">{}</span> · \
         <a href=\"/admin/pages/{}\">edit in admin</a></p>",
        page.revision,
        escape_html(&page.title),
        render_markdown(&page.body),
        page.updated_at,
        escape_html(&page.author),
        escape_html(&page.slug),
    );
    layout(&page.title, chrome, &body, false)
}

/// The API reference page.
pub fn api_docs(chrome: &Chrome, base_url: &str) -> String {
    let body = format!(
        "<article class=\"prose\"><div class=\"kicker\">HTTP API · v1</div>\
         <h1>Exchange API</h1>\
         <p>All endpoints are JSON over HTTPS. Authenticate with \
         <code>Authorization: Bearer &lt;token&gt;</code>. Register with \
         <code>POST /api/v1/accounts</code> and store the returned token — it is shown once.</p>\
         <h2>Public reads</h2>\
         <ul><li><code>GET /api/v1/status</code> — treasury, sequence, ledger root, supply, fiat totals</li>\
         <li><code>GET /api/v1/registry</code> — certificates plus provenance and spent status</li>\
         <li><code>GET /api/v1/denominations</code> — live and expired denominations</li>\
         <li><code>GET /api/v1/ops?limit=50</code> — the signed consensus log</li></ul>\
         <h2>Account lifecycle (sandbox settlement)</h2>\
         <ol><li><code>POST /api/v1/reserves</code> — open a reserve (idempotent)</li>\
         <li><code>POST /api/v1/reserves/&lt;id&gt;/peg-in</code> <code>{{\"amount\":100}}</code> — simulate an \
         incoming bank wire; booked and confirmed in-process</li>\
         <li><code>POST /api/v1/reserves/&lt;id&gt;/withdraw</code> <code>{{\"amount\":100}}</code> — turn reserve \
         credit into an e-coin of a live denomination</li>\
         <li><code>POST /api/v1/deposits</code> <code>{{\"coins\":[\"&lt;id&gt;\"],\"merchant\":\"did:unfer:…\"}}</code> \
         — pay a merchant; the e-coin is retired and the merchant's fiat balance credited</li>\
         <li><code>POST /api/v1/peg-outs</code> <code>{{\"amount\":50,\"bank_account\":\"DE…\"}}</code> — reserve a \
         wire out; an operator confirms it with <code>POST /api/v1/peg-outs/&lt;wire&gt;/confirm</code></li></ol>\
         <h2>Carbon certificates</h2>\
         <p>The treasury mints with <code>POST /api/v1/admin/mint</code> \
         <code>{{\"amount\":10,\"owner\":\"did:unfer:…\",\"source\":\"unfccc:vc:&lt;order&gt;\"}}</code>. \
         The provenance must be a UNFCCC order or a DOI \
         (<code>10.&lt;registrant&gt;/&lt;suffix&gt;</code>); a DOI is what a certificate minted \
         from a published proof carries. Transfers conserve value exactly: \
         <code>POST /api/v1/transfers</code> with inputs and <code>{{\"amount\",\"to\"}}</code> outputs.</p>\
         <h2>Proofs</h2>\
         <p>An operator submits an mm0 development with \
         <code>POST /api/v1/admin/proofs</code> \
         <code>{{\"title\":\"…\",\"mm0\":\"…\",\"mm1\":\"…\",\"project\":\"refi\"}}</code>; \
         it comes back <code>202 queued</code>, <code>verified</code> or <code>rejected</code>. \
         The ledger is public: <code>GET /api/v1/proofs</code> and \
         <code>GET /api/v1/proofs/&lt;id&gt;</code> (the latter also lists the certificates minted \
         from that proof's DOI). Mint from one with \
         <code>POST /api/v1/proofs/&lt;id&gt;/mint</code> \
         <code>{{\"amount\":10,\"owner\":\"did:unfer:…\"}}</code> — the provenance on the \
         mint is the proof's DOI.</p>\
         <h2>Operations</h2>\
         <ul><li><code>GET /api/v1/audit</code> (admin) — the fiat conservation identity</li>\
         <li><code>POST /api/v1/admin/denominations</code> <code>{{\"value\":100,\"expires_seq\":1000}}</code></li>\
         <li><code>GET /metrics</code> — Prometheus text</li>\
         <li><code>GET /healthz</code> — audit + policy liveness</li></ul>\
         <h2>Errors</h2>\
         <pre><code>{{\"error\":{{\"code\":\"UK-7102\",\"name\":\"TalerInsufficientBalance\",\"message\":\"…\"}}}}</code></pre>\
         <p class=\"small muted\">Base URL: <span class=\"mono\">{}</span> · sandbox settlement · no fiat moves.</p>\
         </article>",
        escape_html(base_url)
    );
    layout("API", chrome, &body, false)
}

/// The admin dashboard: content, accounts and the audit identity.
pub fn admin_dashboard(
    chrome: &Chrome,
    pages: &[Page],
    accounts: &[Account],
    status: &Value,
    audit: &Value,
    csrf: &str,
) -> String {
    let rows = pages
        .iter()
        .map(|p| {
            format!(
                "<tr><td><a href=\"/admin/pages/{}\">{}</a></td><td class=\"mono small\">{}</td>\
                 <td><span class=\"pill {}\">{}</span></td><td class=\"num\">r{}</td><td class=\"num\">{}</td>\
                 <td class=\"actions\"><form method=\"post\" action=\"/admin/pages/{}/publish\">\
                 <input type=\"hidden\" name=\"token\" value=\"{csrf}\">\
                 <button class=\"mini\" type=\"submit\">{}</button></form>\
                 <form method=\"post\" action=\"/admin/pages/{}/delete\">\
                 <input type=\"hidden\" name=\"token\" value=\"{csrf}\">\
                 <button class=\"mini danger\" type=\"submit\">delete</button></form></td></tr>",
                escape_html(&p.slug),
                escape_html(&p.title),
                escape_html(&p.slug),
                if p.published { "ok" } else { "muted" },
                if p.published { "published" } else { "draft" },
                p.revision,
                p.updated_at,
                escape_html(&p.slug),
                if p.published { "unpublish" } else { "publish" },
                escape_html(&p.slug),
                csrf = escape_html(csrf),
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let account_rows = accounts
        .iter()
        .map(|a| {
            format!(
                "<tr><td>{}</td><td class=\"mono small\">{}</td><td><span class=\"pill\">{}</span></td><td class=\"num\">{}</td></tr>",
                escape_html(&a.name),
                escape_html(&a.did),
                escape_html(a.role.as_str()),
                a.created_at
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let body = format!(
        "<section class=\"panel\"><div class=\"panel-head\"><h2>Content</h2>\
         <a class=\"button small\" href=\"/admin/proofs\">Proofs</a>\
         <a class=\"button small\" href=\"/admin/pages/new\">New page</a></div>\
         <table><thead><tr><th>Title</th><th>Slug</th><th>State</th><th>Rev</th><th>Updated</th><th></th></tr></thead>\
         <tbody>{rows}</tbody></table></section>\
         <section class=\"panel\"><div class=\"panel-head\"><h2>Exchange</h2><span class=\"small muted\">sandbox</span></div>\
         <div class=\"kv\"><div><span>Seq</span><code>{seq}</code></div>\
         <div><span>Ledger root</span><code class=\"mono small\">{root}</code></div>\
         <div><span>Supply</span><code>{supply}</code></div>\
         <div><span>Audit</span><code class=\"{audit_class}\">{audit_state}</code></div>\
         <div><span>Assets / liabilities</span><code>{assets} / {liabilities}</code></div></div>\
         <form class=\"inline\" method=\"post\" action=\"/admin/denominations\">\
         <input type=\"hidden\" name=\"token\" value=\"{csrf}\">\
         <label>Value <input name=\"value\" type=\"number\" min=\"1\" required></label>\
         <label>Expires seq <input name=\"expires_seq\" type=\"number\" min=\"0\" placeholder=\"horizon\"></label>\
         <button type=\"submit\">Issue denomination</button></form>\
         <form class=\"inline\" method=\"post\" action=\"/admin/accounts\">\
         <input type=\"hidden\" name=\"token\" value=\"{csrf}\">\
         <label>Name <input name=\"name\" required maxlength=\"64\"></label>\
         <label>Role <select name=\"role\"><option>customer</option><option>merchant</option><option>admin</option></select></label>\
         <button type=\"submit\">Create account</button></form>\
         <p class=\"small muted\">Account tokens are shown once, in the response of the request that creates them. \
         The bootstrap admin token lives in <code>admin.token</code> in the state directory.</p></section>\
         <section class=\"panel\"><div class=\"panel-head\"><h2>Accounts</h2><span class=\"small\">{n}</span></div>\
         <table><thead><tr><th>Name</th><th>DID</th><th>Role</th><th>Created</th></tr></thead>\
         <tbody>{account_rows}</tbody></table></section>",
        rows = if rows.is_empty() { "<tr><td colspan=\"6\" class=\"empty\">No pages.</td></tr>".to_string() } else { rows },
        seq = status["seq"].as_u64().unwrap_or(0),
        root = escape_html(status["ledger"]["root"].as_str().unwrap_or("")),
        supply = status["ledger"]["total_supply"].as_u64().unwrap_or(0),
        audit_class = if audit["ok"] == true { "ok-text" } else { "danger-text" },
        audit_state = if audit["ok"] == true { "ok" } else { "BROKEN" },
        assets = audit["assets"].as_u64().unwrap_or(0),
        liabilities = audit["liabilities"].as_u64().unwrap_or(0),
        csrf = escape_html(csrf),
        account_rows = if account_rows.is_empty() { "<tr><td colspan=\"4\" class=\"empty\">No accounts.</td></tr>".to_string() } else { account_rows },
        n = accounts.len(),
    );
    layout("Admin", chrome, &body, true)
}

/// The page editor (new or existing).
pub fn editor(chrome: &Chrome, page: Option<&Page>, csrf: &str, notice: Option<&str>) -> String {
    let (title, slug, body_text, published) = match page {
        Some(p) => (p.title.clone(), p.slug.clone(), p.body.clone(), p.published),
        None => (String::new(), String::new(), String::new(), false),
    };
    let notice_html = notice
        .map(|n| format!("<div class=\"notice\">{}</div>", escape_html(n)))
        .unwrap_or_default();
    let body = format!(
        "{notice_html}<section class=\"panel\"><div class=\"panel-head\"><h2>{heading}</h2>\
         <a class=\"small\" href=\"/admin\">← back</a></div>\
         <form method=\"post\" action=\"/admin/pages\" class=\"stack\">\
         <input type=\"hidden\" name=\"token\" value=\"{csrf}\">\
         <input type=\"hidden\" name=\"slug\" value=\"{slug}\">\
         <label>Title <input name=\"title\" required maxlength=\"160\" value=\"{title}\"></label>\
         <label>Body <textarea name=\"body\" rows=\"18\" spellcheck=\"false\">{body_text}</textarea></label>\
         <label class=\"check\"><input type=\"checkbox\" name=\"published\" value=\"1\" {checked}> Published</label>\
         <p class=\"small muted\">Markdown subset: <code>#</code>/<code>##</code>/<code>###</code> headings, \
         <code>-</code> bullets, <code>**bold**</code>, backtick code, <code>[label](https://…)</code> links. \
         Everything else is escaped.</p>\
         <button type=\"submit\">Save page</button></form></section>",
        heading = if page.is_some() { "Edit page" } else { "New page" },
        csrf = escape_html(csrf),
        slug = escape_html(&slug),
        title = escape_html(&title),
        body_text = escape_html(&body_text),
        checked = if published { "checked" } else { "" },
        notice_html = notice_html,
    );
    layout("Admin · editor", chrome, &body, true)
}

/// The admin login form.
pub fn login(chrome: &Chrome, error: Option<&str>) -> String {
    let err = error
        .map(|e| format!("<div class=\"notice danger\">{}</div>", escape_html(e)))
        .unwrap_or_default();
    let body = format!(
        "<section class=\"panel narrow\"><div class=\"panel-head\"><h2>Admin sign-in</h2></div>{err}\
         <form method=\"post\" action=\"/admin/login\" class=\"stack\">\
         <label>Admin token <input name=\"token\" type=\"password\" required autocomplete=\"current-password\"></label>\
         <button type=\"submit\">Sign in</button></form>\
         <p class=\"small muted\">The token is the contents of <code>admin.token</code> in the service's \
         state directory. It is stored hashed; sign-in compares it in constant time.</p></section>"
    );
    layout("Admin · sign-in", chrome, &body, true)
}

/// A plain "not found" page.
pub fn not_found(chrome: &Chrome) -> String {
    layout(
        "Not found",
        chrome,
        "<section class=\"panel\"><h1>Not found</h1><p>That page does not exist. \
         <a href=\"/\">Return home</a>.</p></section>",
        false,
    )
}

/// Response for a sandbox wire status (used by the admin JSON view).
pub fn wire_status(w: WireStatus) -> &'static str {
    match w {
        WireStatus::Preparing => "preparing",
        WireStatus::Confirmed => "confirmed",
    }
}

pub fn style_css() -> &'static str {
    r#":root{--bg:#0b0f14;--panel:#111820;--line:#1e2a36;--ink:#e6edf3;--muted:#8b9bab;--accent:#34d399;--accent-dim:#0f3d2e;--danger:#f87171;--warn:#fbbf24}
*{box-sizing:border-box}
html{scroll-behavior:smooth}
body{margin:0;background:radial-gradient(1200px 600px at 20% -10%,#12202b 0%,var(--bg) 55%);color:var(--ink);font:16px/1.6 ui-sans-serif,system-ui,-apple-system,"Segoe UI",sans-serif}
a{color:var(--accent);text-decoration:none}a:hover{text-decoration:underline}
.mono,code,pre{font-family:ui-monospace,SFMono-Regular,Menlo,monospace}
.small{font-size:.84rem}.muted{color:var(--muted)}
.banner{background:var(--accent-dim);border-bottom:1px solid #16513d;color:#c7f5e3;text-align:center;padding:.5rem 1rem;font-size:.86rem}
.top{display:flex;align-items:center;justify-content:space-between;gap:1rem;padding:1rem 2rem;border-bottom:1px solid var(--line);position:sticky;top:0;background:rgba(11,15,20,.86);backdrop-filter:blur(8px);z-index:5;flex-wrap:wrap}
.brand{font-weight:700;letter-spacing:-.01em;color:var(--ink)}.mark{color:var(--accent)}
nav{display:flex;gap:1.1rem;align-items:center;flex-wrap:wrap}
nav a{color:var(--muted);font-size:.92rem}nav a:hover{color:var(--ink);text-decoration:none}
.admin-link{color:var(--accent)!important}
main{max-width:1080px;margin:0 auto;padding:2.4rem 1.4rem 4rem}
.hero{padding:2.4rem 0 1.6rem;max-width:780px}
.kicker{color:var(--accent);text-transform:uppercase;letter-spacing:.14em;font-size:.74rem;font-weight:600}
h1{font-size:clamp(2rem,4vw,3rem);line-height:1.1;letter-spacing:-.03em;margin:.6rem 0 1rem}
h2{font-size:1.2rem;margin:0;letter-spacing:-.01em}
.lede{color:var(--muted);font-size:1.06rem;max-width:64ch}
.cta{display:flex;gap:.7rem;margin-top:1.4rem;flex-wrap:wrap}
.button{display:inline-block;padding:.6rem 1.05rem;border-radius:.5rem;background:var(--accent);color:#04150f;font-weight:600;font-size:.92rem;border:1px solid var(--accent)}
.button:hover{text-decoration:none;filter:brightness(1.08)}
.button.ghost{background:transparent;color:var(--ink);border-color:var(--line)}
.stats{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:.9rem;margin:1.6rem 0}
.stat{background:var(--panel);border:1px solid var(--line);border-radius:.7rem;padding:1rem 1.1rem}
.stat-label{color:var(--muted);font-size:.78rem;text-transform:uppercase;letter-spacing:.08em}
.stat-value{font-size:1.8rem;font-weight:700;letter-spacing:-.02em;margin:.2rem 0}
.stat-hint{color:var(--muted);font-size:.8rem}
.panel{background:var(--panel);border:1px solid var(--line);border-radius:.8rem;padding:1.2rem 1.3rem;margin:1.2rem 0}
.panel.narrow{max-width:520px;margin-left:auto;margin-right:auto}
.panel-head{display:flex;justify-content:space-between;align-items:center;gap:1rem;margin-bottom:.9rem;flex-wrap:wrap}
.kv{display:grid;grid-template-columns:repeat(auto-fit,minmax(230px,1fr));gap:.7rem}
.kv>div{border-top:1px solid var(--line);padding-top:.5rem}
.kv span{display:block;color:var(--muted);font-size:.76rem;text-transform:uppercase;letter-spacing:.07em}
.kv code{font-size:.86rem;word-break:break-all}
table{width:100%;border-collapse:collapse;font-size:.9rem}
th{text-align:left;color:var(--muted);font-weight:500;font-size:.76rem;text-transform:uppercase;letter-spacing:.07em;border-bottom:1px solid var(--line);padding:.5rem .4rem}
td{padding:.55rem .4rem;border-bottom:1px solid #16212c;vertical-align:top}
.num{text-align:right;font-variant-numeric:tabular-nums}
.pill{display:inline-block;padding:.1rem .5rem;border-radius:999px;font-size:.72rem;border:1px solid var(--line);color:var(--muted)}
.pill.ok{border-color:#16513d;color:#7ff0c4;background:#0d2a21}
.empty{color:var(--muted);text-align:center;padding:1.2rem}
.actions{display:flex;gap:.4rem;flex-wrap:wrap}.actions form{margin:0}
.mini{padding:.2rem .5rem;font-size:.74rem;border-radius:.35rem;background:#16212c;color:var(--ink);border:1px solid var(--line);cursor:pointer}
.mini.danger{color:var(--danger);border-color:#4a1f22}
button{font:inherit;padding:.55rem 1rem;border-radius:.45rem;border:1px solid var(--accent);background:var(--accent);color:#04150f;font-weight:600;cursor:pointer}
button:hover{filter:brightness(1.08)}
.stack{display:flex;flex-direction:column;gap:.9rem}
label{display:flex;flex-direction:column;gap:.35rem;font-size:.84rem;color:var(--muted)}
input,textarea,select{font:inherit;color:var(--ink);background:#0c141c;border:1px solid var(--line);border-radius:.45rem;padding:.55rem .65rem}
textarea{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.86rem;line-height:1.5;resize:vertical}
label.check{flex-direction:row;align-items:center;gap:.5rem}
label.check input{width:auto}
.inline{display:flex;gap:.7rem;align-items:flex-end;flex-wrap:wrap;margin-top:1rem;border-top:1px solid var(--line);padding-top:1rem}
.prose{max-width:74ch}.prose h2{margin:1.8rem 0 .6rem;font-size:1.35rem}.prose h3{margin:1.4rem 0 .5rem;font-size:1.1rem}
.prose p{margin:.7rem 0}.prose ul{margin:.7rem 0;padding-left:1.2rem}.prose li{margin:.25rem 0}
.prose code{background:#0c141c;border:1px solid var(--line);padding:.06rem .3rem;border-radius:.3rem;font-size:.86em}
.prose pre{background:#0c141c;border:1px solid var(--line);padding:.8rem;border-radius:.5rem;overflow:auto}
.notice{background:#3a2a10;border:1px solid #6b4e1a;color:#fde8b8;padding:.6rem .8rem;border-radius:.5rem;margin-bottom:1rem;font-size:.88rem}
.notice.danger{background:#3a1416;border-color:#6b1a1f;color:#fecaca}
.ok-text{color:var(--accent)}.danger-text{color:var(--danger)}
footer{max-width:1080px;margin:0 auto;padding:1.4rem;border-top:1px solid var(--line);color:var(--muted);font-size:.8rem;display:flex;gap:1rem;justify-content:space-between;flex-wrap:wrap}
@media (max-width:640px){.top{padding:.8rem 1rem}main{padding-top:1.4rem}td,th{font-size:.82rem}}
"#
}


// ── proofs ──────────────────────────────────────────────────────────

fn proof_rows(proofs: &[Proof], admin: bool) -> String {
    let rows = proofs
        .iter()
        .map(|p| {
            let title = if admin {
                format!(
                    "<a href=\"/admin/proofs/{}\">{}</a>",
                    escape_html(&p.id),
                    escape_html(&p.title)
                )
            } else {
                escape_html(&p.title)
            };
            let doi = match (p.doi_url(), p.doi.as_deref()) {
                (Some(url), Some(doi)) => format!(
                    "<a href=\"{}\" rel=\"noopener\">{}</a>",
                    escape_html(&url),
                    escape_html(doi)
                ),
                _ => match p.mm0_proof.as_deref() {
                    Some(id) => format!("<span class=\"mono small\">{}</span>", escape_html(&short(id))),
                    None => "—".to_string(),
                },
            };
            format!(
                "<tr><td>{title}</td><td><span class=\"pill {class}\">{state}</span></td>\
                 <td>{doi}</td><td class=\"mono small\">{sha}</td><td class=\"num\">{at}</td></tr>",
                title = title,
                class = if p.state.as_str() == "published" { "ok" } else { "muted" },
                state = escape_html(p.state.as_str()),
                doi = doi,
                sha = escape_html(&short(&p.sha256)),
                at = p.updated_at,
            )
        })
        .collect::<Vec<_>>()
        .join("");
    if rows.is_empty() {
        "<p class=\"empty\">No proofs yet. An operator submits an mm0 development from the \
         <a href=\"/admin/proofs\">console</a>; it is verified by <code>mm0-rs</code> and \
         <code>mm0-c</code> and archived on Zenodo with a DOI.</p>"
            .to_string()
    } else {
        format!(
            "<table><thead><tr><th>Proof</th><th>State</th><th>DOI</th><th>Digest</th>\
             <th class=\"num\">Updated</th></tr></thead><tbody>{rows}</tbody></table>"
        )
    }
}

fn backend_note(configured: bool, backend: Option<&str>) -> String {
    match (configured, backend) {
        (true, Some(base)) => format!(
            "<p class=\"small\">Verification and archiving run on <code>{}</code>: each \
             development is checked by <code>mm0-rs</code> and then by <code>mm0-c</code> \
             (a warning is a proof hole) before Zenodo stores it and mints a DOI.</p>",
            escape_html(base)
        ),
        _ => "<p class=\"small\">This exchange has no proof backend configured, so \
              submissions are refused. Records already archived stay listed here.</p>"
            .to_string(),
    }
}

/// The public proof ledger: what the exchange proved, where it is archived, and
/// the DOI each record earned.
pub fn proofs(
    chrome: &Chrome,
    proofs: &[Proof],
    counts: (usize, usize, usize),
    configured: bool,
    backend: Option<&str>,
) -> String {
    let (published, pending, failed) = counts;
    let body = format!(
        "<section class=\"panel\"><h1>Proof ledger</h1>{note}\
         <div class=\"stats\">{a}{b}{c}</div>{rows}\
         <p class=\"small\">The machine-readable form is <a href=\"/api/v1/proofs\">\
         /api/v1/proofs</a>.</p></section>",
        note = backend_note(configured, backend),
        a = stat("Archived", &published.to_string(), "with a Zenodo DOI"),
        b = stat("Pending", &pending.to_string(), "queued or verified, not archived"),
        c = stat("Refused", &failed.to_string(), "proof hole or backend error"),
        rows = proof_rows(proofs, false),
    );
    layout("Proofs", chrome, &body, false)
}

/// The console page: submit a development, and see the ledger it lands in.
pub fn admin_proofs(
    chrome: &Chrome,
    proofs: &[Proof],
    counts: (usize, usize, usize),
    configured: bool,
    backend: Option<&str>,
    csrf: &str,
) -> String {
    let form = if configured {
        format!(
            "<form method=\"post\" action=\"/admin/proofs\">\
             <input type=\"hidden\" name=\"token\" value=\"{csrf}\">\
             <p><label>Project <input name=\"project\" size=\"24\" value=\"refi\" required></label></p>\
             <p><label>Title <input name=\"title\" size=\"64\" required></label></p>\
             <p><label>mm0 spec<br><textarea name=\"mm0\" rows=\"8\" cols=\"80\" required></textarea></label></p>\
             <p><label>mm1 proofs<br><textarea name=\"mm1\" rows=\"8\" cols=\"80\"></textarea></label></p>\
             <p><button type=\"submit\">Verify and publish</button></p></form>",
            csrf = escape_html(csrf)
        )
    } else {
        "<p class=\"empty\">Proof publishing is not configured: start the service with \
         <code>--mm0web &lt;url&gt;</code> to reach an mm0web backend.</p>"
            .to_string()
    };
    let (published, pending, failed) = counts;
    let body = format!(
        "<section class=\"panel\"><div class=\"panel-head\"><h2>Submit a proof</h2>\
         <a class=\"small\" href=\"/admin\">← back</a></div>{note}{form}</section>\
         <section class=\"panel\"><h2>Proof ledger</h2>\
         <div class=\"stats\">{a}{b}{c}</div>{rows}</section>",
        note = backend_note(configured, backend),
        form = form,
        a = stat("Archived", &published.to_string(), "with a Zenodo DOI"),
        b = stat("Pending", &pending.to_string(), "queued or verified"),
        c = stat("Refused", &failed.to_string(), "proof hole or backend error"),
        rows = proof_rows(proofs, true),
    );
    layout("Proofs · admin", chrome, &body, true)
}

/// One proof, both views: the admin console (with its mint form) and the public
/// page at `/proofs/<id>`. Same evidence, different toolbar — the log, the
/// archive and the certificates are public because a citation is public.
fn proof_view(
    chrome: &Chrome,
    detail: &ProofDetail,
    csrf: Option<&str>,
    title: &str,
    admin: bool,
) -> String {
    let p = &detail.proof;
    let backend = detail.backend.clone().unwrap_or_else(|| "not configured".to_string());

    let log = match p.log.as_deref() {
        Some(log) if !log.is_empty() => format!("<pre><code>{}</code></pre>", escape_html(log)),
        _ => "<p class=\"empty\">No verification log was recorded for this submission.</p>"
            .to_string(),
    };
    let why = match p.detail.as_deref() {
        Some(d) if !d.is_empty() => format!("<p class=\"small\">{}</p>", escape_html(d)),
        _ => String::new(),
    };
    let archive = if p.can_mint() {
        let doi = p.doi.as_deref().unwrap_or("");
        let record = match (p.url.as_deref(), p.zenodo_record) {
            (Some(url), Some(n)) => format!(
                "<div><span>Record</span><a href=\"{}\" rel=\"noopener\">zenodo record {}</a></div>",
                escape_html(url),
                n
            ),
            _ => String::new(),
        };
        format!(
            "<div class=\"kv\"><div><span>DOI</span><a href=\"https://doi.org/{doi}\" \
             rel=\"noopener\">{doi}</a></div>{record}</div>"
        )
    } else {
        "<p class=\"empty\">Not archived: a proof without a DOI is evidence of checking, \
         not a citation, so it cannot back a certificate.</p>"
            .to_string()
    };

    let cert_rows = detail
        .certificates
        .iter()
        .map(|c| {
            format!(
                "<tr><td class=\"mono small\">{}</td><td class=\"num\">{}</td>\
                 <td class=\"mono small\">{}</td><td><span class=\"pill {}\">{}</span></td></tr>",
                escape_html(c["cert_id"].as_str().unwrap_or("")),
                c["amount"].as_u64().unwrap_or(0),
                escape_html(&short(c["owner"].as_str().unwrap_or(""))),
                if c["status"] == "unspent" { "ok" } else { "muted" },
                escape_html(c["status"].as_str().unwrap_or("")),
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let certificates = if cert_rows.is_empty() {
        "<p class=\"empty\">No certificate has been minted against this proof yet.</p>"
            .to_string()
    } else {
        format!(
            "<table><thead><tr><th>Certificate</th><th class=\"num\">Amount</th><th>Owner</th>\
             <th>Status</th></tr></thead><tbody>{cert_rows}</tbody></table>"
        )
    };

    let mint = match csrf {
        Some(csrf) if p.can_mint() => format!(
            "<form method=\"post\" action=\"/admin/proofs/{id}/mint\">\
             <input type=\"hidden\" name=\"token\" value=\"{csrf}\">\
             <p><label>Amount <input name=\"amount\" size=\"12\" value=\"1\" required></label>\
             <label>Owner <input name=\"owner\" size=\"68\" required></label></p>\
             <p class=\"small\">The provenance on the mint is the DOI above; the policy \
             kernel still gates the amount and the audit.</p>\
             <p><button type=\"submit\">Mint certificate from this proof</button></p></form>",
            id = escape_html(&p.id),
            csrf = escape_html(csrf),
        ),
        _ => String::new(),
    };
    let back = if admin {
        "<a class=\"small\" href=\"/admin/proofs\">← proofs</a>".to_string()
    } else {
        "<a class=\"small\" href=\"/proofs\">← proofs</a>".to_string()
    };
    let who = if admin {
        format!("<div><span>Submitted by</span><code>{}</code></div>", escape_html(&p.submitted_by))
    } else {
        String::new()
    };

    let body = format!(
        "<section class=\"panel\"><div class=\"panel-head\"><h2>{heading}</h2>{back}</div>\
         <p>State: <span class=\"pill {class}\">{state}</span></p>\
         <div class=\"kv\"><div><span>Id</span><code>{id}</code></div>\
         <div><span>Digest</span><code>{sha}</code></div>\
         <div><span>Project</span><code>{project}</code></div>\
         <div><span>mm0web proof</span><code>{mm0}</code></div>\
         <div><span>Backend</span><code>{backend}</code></div>{who}</div>\
         <h3>Verification log</h3>{log}{why}<h3>Archive</h3>{archive}</section>\
         <section class=\"panel\"><div class=\"panel-head\"><h2>Certificates from this proof</h2>\
         <span class=\"small\">{n}</span></div>{certificates}{mint}</section>",
        heading = escape_html(title),
        back = back,
        class = if p.state.as_str() == "published" { "ok" } else { "muted" },
        state = escape_html(p.state.as_str()),
        id = escape_html(&p.id),
        sha = escape_html(&p.sha256),
        project = escape_html(&p.project),
        mm0 = escape_html(p.mm0_proof.as_deref().unwrap_or("\u{2014}")),
        backend = escape_html(&backend),
        who = who,
        log = log,
        why = why,
        archive = archive,
        certificates = certificates,
        n = detail.certificates.len(),
        mint = mint,
    );
    layout(title, chrome, &body, admin)
}

/// The console: what the checkers said, where the bytes are archived, and the
/// certificates minted against that archive.
pub fn admin_proof_detail(chrome: &Chrome, detail: &ProofDetail, csrf: &str) -> String {
    proof_view(chrome, detail, Some(csrf), "Proof \u{b7} admin", true)
}

/// The public page for one proof — the target of the registry's "backed by"
/// link, so a reader can walk from a certificate to the proof that justifies it.
pub fn proof_page(chrome: &Chrome, detail: &ProofDetail) -> String {
    proof_view(chrome, detail, None, &detail.proof.title, false)
}

/// What the console shows right after a submission: the state, the digest and
/// — when it exists already — the DOI.
pub fn proof_result(chrome: &Chrome, proof: &Proof) -> String {
    let doi = match proof.doi_url() {
        Some(url) => format!(
            "<div><span>DOI</span><a href=\"{}\">{}</a></div>",
            escape_html(&url),
            escape_html(proof.doi.as_deref().unwrap_or(""))
        ),
        None => "<div><span>DOI</span><code>not archived yet</code></div>".to_string(),
    };
    let detail = match proof.detail.as_deref() {
        Some(d) if !d.is_empty() => format!("<pre><code>{}</code></pre>", escape_html(d)),
        _ => String::new(),
    };
    let body = format!(
        "<section class=\"panel\"><div class=\"panel-head\"><h2>{title}</h2>\
         <a class=\"small\" href=\"/admin/proofs\">← proofs</a></div>\
         <p>State: <span class=\"pill {class}\">{state}</span></p>\
         <div class=\"kv\"><div><span>Id</span><code>{id}</code></div>\
         <div><span>Digest</span><code>{sha}</code></div>{doi}\
         <div><span>mm0web proof</span><code>{mm0}</code></div></div>{detail}</section>",
        title = escape_html(&proof.title),
        class = if proof.state.as_str() == "published" { "ok" } else { "muted" },
        state = escape_html(proof.state.as_str()),
        id = escape_html(&proof.id),
        sha = escape_html(&proof.sha256),
        doi = doi,
        mm0 = escape_html(proof.mm0_proof.as_deref().unwrap_or("—")),
        detail = detail,
    );
    layout("Proof submitted", chrome, &body, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chrome() -> Chrome<'static> {
        Chrome {
            pages: &[],
            sandbox: true,
            base_url: "https://example.test",
        }
    }

    #[test]
    fn pages_escape_titles_and_bodies() {
        let page = Page {
            slug: "x".into(),
            title: "<script>x</script>".into(),
            body: "**hi** <img src=x onerror=alert(1)>".into(),
            published: true,
            updated_at: 1,
            author: "a".into(),
            revision: 1,
        };
        let html = page_view(&chrome(), &page);
        assert!(!html.contains("<script>x</script>"));
        assert!(!html.contains("<img src=x"));
        assert!(html.contains("&lt;img"));
    }

    #[test]
    fn home_shows_the_sandbox_banner_and_numbers() {
        let status = serde_json::json!({
            "seq": 7,
            "ledger": {"root": "ab".repeat(32), "total_supply": 42, "unspent": 3},
            "fiat": {"in": 100, "out": 10},
            "reserves_total": 20,
            "merchants_total": 30,
            "funded_outstanding": 40,
            "treasury_did": "did:unfer:aa",
        });
        let registry = serde_json::json!({"certificates": []});
        let html = home(&chrome(), &status, &registry);
        assert!(html.contains("Sandbox settlement"));
        assert!(html.contains(">42<"));
        assert!(html.contains("No certificates minted yet"));
    }
}
