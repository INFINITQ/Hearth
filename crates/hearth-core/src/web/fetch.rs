//! Reading a web page: its main text (Readability) as Markdown, cut to a size.
//!
//! Unless Settings → Web allows it, pages on this PC or the local network aren't read: every
//! address (and every redirect) is checked after its name is looked up, and the connection
//! goes to the checked address, so a page can't point the tools at a router or a local server.

use std::net::{IpAddr, SocketAddr};
use std::sync::LazyLock;
use std::time::Duration;

use dom_query::Document;
use dom_smoothie::{Config, Readability, TextMode};
use futures_util::StreamExt;
use regex::Regex;
use reqwest::Url;
use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};

use super::{USER_AGENT, WebError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// Where the page ended up (after redirects).
    pub url: String,
    pub title: String,
    pub markdown: String,
    /// More of the page was left out.
    pub truncated: bool,
}

/// Bigger pages are cut (their start is what matters).
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

/// An address on the public internet (not this PC, the local network or a reserved range).
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || a == 0
                || a >= 240
                // Carrier-grade NAT and benchmarking ranges.
                || (a == 100 && (b & 0xc0) == 64)
                || (a == 198 && (b & 0xfe) == 18))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || v6.is_unique_local()
                || v6.is_unicast_link_local())
        }
    }
}

fn parse(url: &str) -> Result<Url, WebError> {
    let url = Url::parse(url.trim())
        .map_err(|_| WebError::Refused(format!("“{url}” isn't a web address.")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(WebError::Refused(format!(
            "Only http and https pages can be read, not {}: addresses.",
            url.scheme()
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(WebError::Refused(
            "Addresses with a user name or password aren't read.".into(),
        ));
    }
    Ok(url)
}

/// The addresses to connect to, checked.
async fn addresses(url: &Url, allow_local: bool) -> Result<Vec<SocketAddr>, WebError> {
    let host = url
        .host_str()
        .ok_or_else(|| WebError::Refused(format!("{url} has no host.")))?;
    let port = url.port_or_known_default().unwrap_or(443);
    // `[::1]` style hosts are looked up without their brackets.
    let lookup = host.trim_start_matches('[').trim_end_matches(']');
    let found: Vec<SocketAddr> = tokio::net::lookup_host((lookup, port))
        .await
        .map_err(|_| WebError::Network(format!("{host} wasn't found.")))?
        .collect();
    if found.is_empty() {
        return Err(WebError::Network(format!("{host} wasn't found.")));
    }
    if !allow_local && found.iter().any(|addr| !is_public(addr.ip())) {
        return Err(WebError::Refused(format!(
            "{host} is on this PC or your local network, which web tools don't read. Settings → Web can allow it."
        )));
    }
    Ok(found)
}

pub(super) async fn fetch(
    url: &str,
    max_chars: usize,
    allow_local: bool,
) -> Result<Page, WebError> {
    let mut url = parse(url)?;
    for _ in 0..=MAX_REDIRECTS {
        let found = addresses(&url, allow_local).await?;
        let host = url.host_str().unwrap_or_default().to_owned();
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(25))
            .resolve_to_addrs(&host, &found)
            .build()
            .map_err(|err| WebError::Network(err.to_string()))?;
        let response = client
            .get(url.clone())
            .header(
                ACCEPT,
                "text/html,application/xhtml+xml,text/plain;q=0.9,application/json;q=0.8,*/*;q=0.5",
            )
            .send()
            .await
            .map_err(|err| {
                if err.is_timeout() {
                    WebError::Network(format!("{host} took too long to answer."))
                } else {
                    WebError::Network(format!("Couldn't reach {host}: {err}"))
                }
            })?;
        let status = response.status();
        if status.is_redirection() {
            let next = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|location| url.join(location).ok())
                .ok_or_else(|| {
                    WebError::Refused(format!("{host} redirected without saying where."))
                })?;
            url = parse(next.as_str())?;
            continue;
        }
        if !status.is_success() {
            return Err(WebError::Refused(format!(
                "{host} answered with an error ({status})."
            )));
        }
        return read(url, response, max_chars).await;
    }
    Err(WebError::Refused(
        "The page redirected too many times.".into(),
    ))
}

async fn read(url: Url, response: reqwest::Response, max_chars: usize) -> Result<Page, WebError> {
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned();
    if mime == "application/pdf" {
        return Err(WebError::Refused(
            "This is a PDF, which Hearth can't read from the web yet.".into(),
        ));
    }
    let textual = mime.is_empty()
        || mime.starts_with("text/")
        || mime.contains("html")
        || mime.contains("json")
        || mime.contains("xml")
        || mime.contains("javascript");
    if !textual {
        return Err(WebError::Refused(format!(
            "This is a {mime} file, not a page."
        )));
    }
    let mut bytes = Vec::new();
    let mut cut = false;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|err| WebError::Network(format!("The page stopped loading: {err}")))?;
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_BYTES {
            bytes.truncate(MAX_BYTES);
            cut = true;
            break;
        }
    }
    let text = decode(&bytes, &content_type);
    let html = mime.contains("html") || (mime.is_empty() && looks_like_html(&text));
    let (title, markdown) = if html {
        readable_markdown(&text, url.as_str())
    } else {
        (url.to_string(), text)
    };
    let (markdown, shortened) = shorten(&tidy(&markdown), max_chars);
    Ok(Page {
        url: url.to_string(),
        title: if title.trim().is_empty() {
            url.to_string()
        } else {
            title.trim().to_owned()
        },
        markdown,
        truncated: cut || shortened,
    })
}

static META_CHARSET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)<meta[^>]+charset\s*=\s*["']?\s*([\w-]+)"#).expect("a valid regex")
});

/// Text in the page's own encoding (the header's, else the page's `<meta charset>`, else UTF-8).
fn decode(bytes: &[u8], content_type: &str) -> String {
    let from_header = content_type
        .split(';')
        .filter_map(|part| part.trim().strip_prefix("charset="))
        .next()
        .map(|label| label.trim_matches('"').to_owned());
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]);
    let from_meta = META_CHARSET
        .captures(&head)
        .map(|captures| captures[1].to_owned());
    let encoding = from_header
        .or(from_meta)
        .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    encoding.decode(bytes).0.into_owned()
}

fn looks_like_html(text: &str) -> bool {
    let start: String = text
        .chars()
        .take(512)
        .collect::<String>()
        .to_ascii_lowercase();
    start.contains("<!doctype html") || start.contains("<html")
}

/// The page's title and main text as Markdown. Pages that aren't articles (search pages,
/// apps) fall back to all their visible text.
pub fn readable_markdown(html: &str, url: &str) -> (String, String) {
    let config = Config {
        text_mode: TextMode::Markdown,
        ..Default::default()
    };
    let article = Readability::new(html, Some(url), Some(config)).and_then(|mut r| r.parse());
    if let Ok(article) = article
        && article.text_content.trim().chars().count() >= 200
    {
        return (article.title, plainer(&article.text_content));
    }
    let document = Document::from(html);
    let title = document.select("title").first().text().trim().to_owned();
    document
        .select("script, style, noscript, template, svg, nav, header, footer, form, iframe")
        .remove();
    let body = document.select("body");
    let text = if body.exists() {
        body.formatted_text()
    } else {
        document.formatted_text()
    };
    (title, text.to_string())
}

static IMAGES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"!\[[^\]]*\]\([^)]*\)").expect("a valid regex"));
static ESCAPES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\([.!()\-_#+])").expect("a valid regex"));

/// Markdown without images (icons, mostly) or escapes the model doesn't need (`end\.`).
fn plainer(markdown: &str) -> String {
    let markdown = IMAGES.replace_all(markdown, "");
    ESCAPES.replace_all(&markdown, "$1").into_owned()
}

static BLANK_LINES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\n[ \t]*(\n[ \t]*){2,}").expect("a valid regex"));

/// No runs of blank lines or trailing spaces.
fn tidy(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    BLANK_LINES
        .replace_all(lines.join("\n").trim(), "\n\n")
        .into_owned()
}

/// At most `max` characters, ending at a paragraph or line when one is close.
fn shorten(text: &str, max: usize) -> (String, bool) {
    let Some((cut, _)) = text.char_indices().nth(max) else {
        return (text.to_owned(), false);
    };
    let head = &text[..cut];
    let end = head
        .rfind("\n\n")
        .filter(|&at| at > cut * 3 / 4)
        .or_else(|| head.rfind('\n').filter(|&at| at > cut * 3 / 4))
        .unwrap_or(cut);
    (head[..end].trim_end().to_owned(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_addresses_count_as_the_internet() {
        for local in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.9",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:192.168.0.1",
        ] {
            assert!(!is_public(local.parse().unwrap()), "{local} is local");
        }
        for public in ["93.184.216.34", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(is_public(public.parse().unwrap()), "{public} is public");
        }
    }

    #[test]
    fn only_plain_web_addresses_are_read() {
        assert!(parse("https://example.com/a?b=c").is_ok());
        assert!(parse("file:///C:/Windows/win.ini").is_err());
        assert!(parse("https://user:pw@example.com/").is_err());
        assert!(parse("not a url").is_err());
    }

    #[tokio::test]
    async fn local_pages_are_refused_unless_allowed() {
        let err = fetch("http://127.0.0.1:9/", 1000, false).await.unwrap_err();
        assert!(matches!(err, WebError::Refused(_)), "{err:?}");
        let err = fetch("http://localhost:9/", 1000, false).await.unwrap_err();
        assert!(matches!(err, WebError::Refused(_)), "{err:?}");
    }

    #[test]
    fn an_article_reads_as_markdown_without_the_page_around_it() {
        let paragraph = "Hearth keeps the conversation in a tree of messages, so editing an earlier \
            message starts a new branch and the old one stays.";
        let html = format!(
            "<!doctype html><html><head><title>How branches work</title></head><body>\
             <nav><a href='/'>Home</a> <a href='/docs'>Docs</a></nav>\
             <article><h1>How branches work</h1><p>{paragraph}</p><p>{paragraph}</p>\
             <ul><li>One</li><li>Two</li></ul><p>{paragraph}</p></article>\
             <footer>Copyright</footer><script>track()</script></body></html>"
        );
        let (title, markdown) = readable_markdown(&html, "https://docs.example/branches");
        assert_eq!(title, "How branches work");
        assert!(
            markdown.contains("editing an earlier message"),
            "{markdown}"
        );
        assert!(!markdown.contains("track()"));
        assert!(!markdown.contains("Copyright"));
        assert!(!markdown.contains("\\."), "{markdown}");
        assert_eq!(
            plainer("Done\\! See ![icon](/i.svg)[the book](https://b.example/)\\."),
            "Done! See [the book](https://b.example/)."
        );
    }

    #[test]
    fn pages_that_arent_articles_keep_their_visible_text() {
        let html = "<html><head><title>Status</title><style>p{}</style></head>\
                    <body><p>All systems normal.</p><script>x()</script></body></html>";
        let (title, text) = readable_markdown(html, "https://status.example/");
        assert_eq!(title, "Status");
        assert!(text.contains("All systems normal."));
        assert!(!text.contains("x()"));
    }

    #[test]
    fn text_is_decoded_in_the_pages_own_encoding() {
        let latin1 = b"<html><head><meta charset=\"iso-8859-1\"></head><body>caf\xe9</body></html>";
        assert!(decode(latin1, "text/html").contains("café"));
        assert!(decode("café".as_bytes(), "text/plain; charset=utf-8").contains("café"));
    }

    #[test]
    fn long_pages_are_cut_at_a_paragraph() {
        let text = format!("{}\n\n{}", "a".repeat(900), "b".repeat(900));
        let (short, cut) = shorten(&text, 1000);
        assert!(cut);
        assert_eq!(short, "a".repeat(900));
        assert_eq!(shorten("short", 1000), ("short".to_owned(), false));
        assert_eq!(tidy("a  \n\n\n\nb"), "a\n\nb");
    }
}
