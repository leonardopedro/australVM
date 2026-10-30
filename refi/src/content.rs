//! The content model behind the public site: typed pages with a slug, a
//! draft/published state and an audit stamp, plus a deliberately small
//! Markdown subset renderer.
//!
//! This is the EmDash-shaped part of the service — a typed schema, an admin
//! editor and published-only public rendering — implemented in Rust and served
//! by the same HTTP front as the API, with no second runtime and no second
//! deployment target. Everything user-supplied is HTML-escaped *before* the
//! Markdown subset is applied, so a page body cannot inject markup beyond the
//! tags this renderer emits itself.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::util::{escape_html, now_unix, slugify};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub slug: String,
    pub title: String,
    pub body: String,
    /// Drafts are visible in the admin console and to its author (as a
    /// preview) but never listed or served on the public site.
    pub published: bool,
    pub updated_at: u64,
    /// Who last saved it (a DID, or "bootstrap" for the seeded pages).
    pub author: String,
    /// Monotonic revision, so an operator can tell two saves apart.
    pub revision: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ContentStore {
    #[serde(default)]
    pub pages: Vec<Page>,
    /// Whether the seed pages were installed; seeded once, then never again,
    /// so an operator's edits survive a restart.
    #[serde(default)]
    pub seeded: bool,
}

impl ContentStore {
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("content file {} is not valid: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("read {}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| format!("encode content: {e}"))?;
        crate::util::write_atomic(path, &bytes, false)
    }

    /// Install the first-run pages (once).
    pub fn seed_if_empty(&mut self) {
        if self.seeded {
            return;
        }
        self.seeded = true;
        let pages = [
            (
                "about",
                "About this exchange",
                "This service is the web front of the unfer ReFi exchange: GNU Taler-style \
                 settlement (reserves, withdraw, deposit, peg-out) over the unfer \
                 carbon-certificate ledger.\n\n\
                 Carbon certificates are minted against a UNFCCC provenance reference and \
                 settle through the same e-coin ledger as fiat-backed value. Every state \
                 transition is signed and replayable, so an independent node can recompute \
                 the ledger root.\n\n\
                 ## What the ledger guarantees\n\n\
                 - Minting requires the configured mint authority.\n\
                 - A transfer conserves total value exactly.\n\
                 - A coin has exactly one spend (nullifier bookkeeping).\n\
                 - The fiat side never leaks: `fiat_in - fiat_out` always equals reserves + \
                 merchant balances + outstanding e-coins.\n",
            ),
            (
                "sandbox",
                "Sandbox settlement notice",
                "**This deployment runs sandbox settlement.** Bank wires are simulated by the \
                 service itself: a peg-in is booked and confirmed in-process, and a peg-out \
                 is a reserved wire an operator confirms. No fiat moves. Denominations and \
                 amounts are integers in the sandbox unit, and no real-world money, \
                 securities or carbon instruments change hands.\n\n\
                 The regulatory surface of a live deployment (VASP registration, custody, \
                 commodity-backed e-coin treatment) is out of scope here and is unchanged \
                 from the project plan.\n",
            ),
            (
                "registry",
                "How to read the registry",
                "Each row is a certificate minted by the treasury. `source` is the provenance \
                 reference recorded on the consensus op — for carbon certificates that is a \
                 UNFCCC cancellation reference. `status` is derived by replaying the op log \
                 against the current unspent set: a spent row is history, an unspent row is a \
                 live certificate.\n\n\
                 The ledger root shown on the home page is the sparse-Merkle commitment over \
                 the unspent set, so two nodes that replay the same log agree on it.\n",
            ),
        ];
        for (slug, title, body) in pages {
            self.pages.push(Page {
                slug: slug.to_string(),
                title: title.to_string(),
                body: body.to_string(),
                published: true,
                updated_at: now_unix(),
                author: "bootstrap".to_string(),
                revision: 1,
            });
        }
    }

    pub fn by_slug(&self, slug: &str) -> Option<&Page> {
        self.pages.iter().find(|p| p.slug == slug)
    }

    pub fn published(&self) -> Vec<&Page> {
        let mut out: Vec<&Page> = self.pages.iter().filter(|p| p.published).collect();
        out.sort_by(|a, b| a.slug.cmp(&b.slug));
        out
    }

    pub fn public_slugs(&self) -> Vec<(String, String)> {
        self.published()
            .into_iter()
            .map(|p| (p.slug.clone(), p.title.clone()))
            .collect()
    }

    /// Create or update a page. A new page gets slug = slugify(title) unless a
    /// slug is given; `revision` increments on every save.
    pub fn put(&mut self, slug: Option<&str>, title: &str, body: &str, published: bool, author: &str) -> Page {
        let slug = match slug {
            Some(s) if !s.trim().is_empty() => slugify(s),
            _ => slugify(title),
        };
        match self.pages.iter_mut().find(|p| p.slug == slug) {
            Some(page) => {
                page.title = title.to_string();
                page.body = body.to_string();
                page.published = published;
                page.updated_at = now_unix();
                page.author = author.to_string();
                page.revision += 1;
                page.clone()
            }
            None => {
                let page = Page {
                    slug,
                    title: title.to_string(),
                    body: body.to_string(),
                    published,
                    updated_at: now_unix(),
                    author: author.to_string(),
                    revision: 1,
                };
                self.pages.push(page.clone());
                page
            }
        }
    }

    pub fn delete(&mut self, slug: &str) -> bool {
        let before = self.pages.len();
        self.pages.retain(|p| p.slug != slug);
        self.pages.len() != before
    }
}

pub fn content_path(state_dir: &Path) -> PathBuf {
    state_dir.join("content.json")
}

/// Render the Markdown subset the editor offers: `#`/`##`/`###` headings,
/// `-` bullets, blank-line-separated paragraphs, `**bold**`, `` `code` `` and
/// `[text](https://…)` links. Input is escaped first; only this renderer emits
/// markup.
pub fn render_markdown(body: &str) -> String {
    let mut out = String::new();
    let mut in_list = false;
    for raw in body.lines() {
        let line = raw.trim_end();
        let escaped = escape_html(line);
        if let Some(rest) = escaped.strip_prefix("### ") {
            close_list(&mut out, &mut in_list);
            out.push_str(&format!("<h4>{}</h4>", inline(rest)));
        } else if let Some(rest) = escaped.strip_prefix("## ") {
            close_list(&mut out, &mut in_list);
            out.push_str(&format!("<h3>{}</h3>", inline(rest)));
        } else if let Some(rest) = escaped.strip_prefix("# ") {
            close_list(&mut out, &mut in_list);
            out.push_str(&format!("<h2>{}</h2>", inline(rest)));
        } else if let Some(rest) = escaped.strip_prefix("- ") {
            if !in_list {
                out.push_str("<ul>");
                in_list = true;
            }
            out.push_str(&format!("<li>{}</li>", inline(rest)));
        } else if escaped.trim().is_empty() {
            close_list(&mut out, &mut in_list);
        } else {
            close_list(&mut out, &mut in_list);
            out.push_str(&format!("<p>{}</p>", inline(&escaped)));
        }
    }
    close_list(&mut out, &mut in_list);
    out
}

fn close_list(out: &mut String, in_list: &mut bool) {
    if *in_list {
        out.push_str("</ul>");
        *in_list = false;
    }
}

/// Inline formatting over already-escaped text.
fn inline(escaped: &str) -> String {
    let mut s = escaped.to_string();
    // Links: only http(s), and the label/URL are already escaped.
    while let Some(start) = s.find('[') {
        let Some(mid) = s[start..].find("](") else { break };
        let mid = start + mid;
        let Some(end_rel) = s[mid + 2..].find(')') else { break };
        let end = mid + 2 + end_rel;
        let label = s[start + 1..mid].to_string();
        let url = s[mid + 2..end].to_string();
        if url.starts_with("https://") || url.starts_with("http://") {
            let link = format!("<a href=\"{url}\" rel=\"noopener noreferrer\">{label}</a>");
            s = format!("{}{}{}", &s[..start], link, &s[end + 1..]);
        } else {
            // Unsafe scheme: leave the source text visible, drop the link form.
            s = format!("{}{}{}", &s[..start], label, &s[end + 1..]);
        }
    }
    bold_and_code(&s)
}

/// Apply `**bold**` and `` `code` `` spans. Both markers were escaped already
/// (they contain no HTML-special characters).
fn bold_and_code(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let bold = rest.find("**");
        let code = rest.find('`');
        let next = match (bold, code) {
            (Some(b), Some(c)) => Some(if b < c { (b, "**", "<strong>", "</strong>") } else { (c, "`", "<code>", "</code>") }),
            (Some(b), None) => Some((b, "**", "<strong>", "</strong>")),
            (None, Some(c)) => Some((c, "`", "<code>", "</code>")),
            (None, None) => None,
        };
        let Some((idx, marker, open, close)) = next else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..idx]);
        let after = &rest[idx + marker.len()..];
        match after.find(marker) {
            Some(close_idx) => {
                out.push_str(open);
                out.push_str(&after[..close_idx]);
                out.push_str(close);
                rest = &after[close_idx + marker.len()..];
            }
            None => {
                // Unbalanced marker: emit it literally.
                out.push_str(marker);
                rest = after;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_subset_renders_and_cannot_inject_markup() {
        let html = render_markdown(
            "# Title\n\nA **bold** and `code` line with [a link](https://example.com).\n\n- one\n- two\n\n<script>alert(1)</script>\n",
        );
        assert!(html.contains("<h2>Title</h2>"));
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<code>code</code>"));
        assert!(html.contains("<a href=\"https://example.com\" rel=\"noopener noreferrer\">a link</a>"));
        assert!(html.contains("<li>one</li>"));
        assert!(!html.contains("<script>"), "raw script tags must be escaped: {html}");
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn unsafe_link_schemes_are_not_linked() {
        let html = render_markdown("[click](javascript:alert(1))");
        assert!(!html.contains("href"));
        assert!(html.contains("click"));
    }

    #[test]
    fn pages_round_trip_and_slugify() {
        let dir = tempfile::tempdir().unwrap();
        let path = content_path(dir.path());
        let mut store = ContentStore::default();
        store.seed_if_empty();
        let seeded = store.pages.len();
        assert!(seeded >= 3);
        store.seed_if_empty(); // idempotent
        assert_eq!(store.pages.len(), seeded);

        let page = store.put(None, "My New Page!", "body v1", false, "did:unfer:admin");
        assert_eq!(page.slug, "my-new-page");
        assert!(!page.published);
        assert_eq!(store.published().len(), seeded, "drafts are not public");

        let updated = store.put(Some("my-new-page"), "My New Page!", "body v2", true, "did:unfer:admin");
        assert_eq!(updated.revision, 2);
        assert!(store.published().iter().any(|p| p.slug == "my-new-page"));
        store.save(&path).unwrap();
        let mut loaded = ContentStore::load(&path).unwrap();
        assert!(loaded.by_slug("my-new-page").unwrap().body.contains("v2"));
        assert!(loaded.delete("my-new-page"));
        assert!(!loaded.delete("my-new-page"));
    }
}
