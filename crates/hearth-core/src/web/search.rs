//! Search providers. Each gives the same hits: a title, an address and a short snippet.

use std::collections::HashSet;
use std::sync::LazyLock;

use dom_query::{Document, Selection};
use regex::Regex;
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{SearchProvider, WebError};
use crate::settings::WebSettings;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

pub(super) struct Keys {
    pub brave: Option<String>,
    pub tavily: Option<String>,
}

const SNIPPET_CHARS: usize = 320;

pub(super) async fn search(
    client: &reqwest::Client,
    web: &WebSettings,
    keys: &Keys,
    query: &str,
    count: usize,
) -> Result<Vec<SearchHit>, WebError> {
    let hits = match web.provider {
        SearchProvider::DuckDuckGo => duckduckgo(client, query).await?,
        SearchProvider::Brave => brave(client, keys.brave.as_deref(), query, count).await?,
        SearchProvider::Tavily => tavily(client, keys.tavily.as_deref(), query, count).await?,
        SearchProvider::Searxng => searxng(client, web.searxng_url.as_deref(), query).await?,
    };
    Ok(clean(hits, count))
}

fn network(provider: SearchProvider) -> impl Fn(reqwest::Error) -> WebError {
    move |err| {
        if err.is_timeout() {
            WebError::Network(format!("{} took too long to answer.", provider.name()))
        } else {
            WebError::Network(format!("Couldn't reach {}: {err}", provider.name()))
        }
    }
}

/// What an error status from a provider means for the user.
fn refused(provider: SearchProvider, status: StatusCode) -> WebError {
    let name = provider.name();
    match status.as_u16() {
        401 | 403 => WebError::Setup(format!(
            "{name} refused the request. Check the API key in Settings → Web."
        )),
        429 => WebError::Setup(format!(
            "{name}'s limit was reached. Try again in a minute, or pick another provider in Settings → Web."
        )),
        _ => WebError::Network(format!("{name} answered with an error ({status}).")),
    }
}

/// `query` form-encoded, as in a query string or a form body.
fn encode(pairs: &[(&str, &str)]) -> String {
    let mut url = Url::parse("https://hearth.invalid/").expect("a valid URL");
    url.query_pairs_mut().extend_pairs(pairs);
    url.query().unwrap_or_default().to_owned()
}

async fn duckduckgo(client: &reqwest::Client, query: &str) -> Result<Vec<SearchHit>, WebError> {
    let provider = SearchProvider::DuckDuckGo;
    let response = client
        .post("https://html.duckduckgo.com/html/")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Referer", "https://html.duckduckgo.com/")
        .body(encode(&[("q", query), ("b", ""), ("kl", "")]))
        .send()
        .await
        .map_err(network(provider))?;
    let status = response.status();
    let html = response.text().await.map_err(network(provider))?;
    if status == StatusCode::ACCEPTED
        || html.contains("anomaly-modal")
        || html.contains("bots use DuckDuckGo")
    {
        return Err(WebError::Setup(
            "DuckDuckGo wants to check that a person is searching, so Hearth can't search there for \
             a while. Try again later, or pick Brave, Tavily or SearXNG in Settings → Web."
                .into(),
        ));
    }
    if !status.is_success() {
        return Err(refused(provider, status));
    }
    Ok(parse_duckduckgo(&html))
}

/// The real address behind one of DuckDuckGo's redirect links.
fn duckduckgo_target(href: &str) -> Option<String> {
    let absolute = if href.starts_with("//") {
        format!("https:{href}")
    } else if href.starts_with('/') {
        format!("https://duckduckgo.com{href}")
    } else {
        href.to_owned()
    };
    let url = Url::parse(&absolute).ok()?;
    let host = url.host_str().unwrap_or_default();
    if host.ends_with("duckduckgo.com") {
        // Redirect links carry the target; other DuckDuckGo links are ads or its own pages.
        return url
            .query_pairs()
            .find(|(key, _)| key == "uddg")
            .map(|(_, value)| value.into_owned());
    }
    Some(url.to_string())
}

/// Results from DuckDuckGo's HTML page (ads left out).
pub fn parse_duckduckgo(html: &str) -> Vec<SearchHit> {
    let document = Document::from(html);
    let mut hits = Vec::new();
    for node in document.select("div.result").nodes() {
        let result = Selection::from(*node);
        if result.has_class("result--ad") {
            continue;
        }
        let link = result.select("a.result__a").first();
        let Some(url) = link.attr("href").and_then(|href| duckduckgo_target(&href)) else {
            continue;
        };
        hits.push(SearchHit {
            title: link.text().to_string(),
            url,
            snippet: result.select(".result__snippet").first().text().to_string(),
        });
    }
    hits
}

async fn json_of(provider: SearchProvider, response: reqwest::Response) -> Result<Value, WebError> {
    let status = response.status();
    if !status.is_success() {
        return Err(refused(provider, status));
    }
    response.json::<Value>().await.map_err(|err| {
        WebError::Network(format!(
            "{} sent an answer Hearth can't read: {err}",
            provider.name()
        ))
    })
}

fn need_key(provider: SearchProvider, key: Option<&str>) -> Result<&str, WebError> {
    key.ok_or_else(|| {
        WebError::Setup(format!(
            "{} needs an API key: add it in Settings → Web, or pick another provider there.",
            provider.name()
        ))
    })
}

async fn brave(
    client: &reqwest::Client,
    key: Option<&str>,
    query: &str,
    count: usize,
) -> Result<Vec<SearchHit>, WebError> {
    let provider = SearchProvider::Brave;
    let key = need_key(provider, key)?;
    let count = count.to_string();
    let url = format!(
        "https://api.search.brave.com/res/v1/web/search?{}",
        encode(&[("q", query), ("count", &count)])
    );
    let response = client
        .get(url)
        .header("Accept", "application/json")
        .header("X-Subscription-Token", key)
        .send()
        .await
        .map_err(network(provider))?;
    Ok(hits_from(
        &json_of(provider, response).await?["web"]["results"],
        "description",
    ))
}

async fn tavily(
    client: &reqwest::Client,
    key: Option<&str>,
    query: &str,
    count: usize,
) -> Result<Vec<SearchHit>, WebError> {
    let provider = SearchProvider::Tavily;
    let key = need_key(provider, key)?;
    let response = client
        .post("https://api.tavily.com/search")
        .bearer_auth(key)
        .json(&json!({ "query": query, "max_results": count, "search_depth": "basic" }))
        .send()
        .await
        .map_err(network(provider))?;
    Ok(hits_from(
        &json_of(provider, response).await?["results"],
        "content",
    ))
}

async fn searxng(
    client: &reqwest::Client,
    base: Option<&str>,
    query: &str,
) -> Result<Vec<SearchHit>, WebError> {
    let provider = SearchProvider::Searxng;
    let base = base
        .map(str::trim)
        .filter(|base| !base.is_empty())
        .ok_or_else(|| {
            WebError::Setup("Add your SearXNG server's address in Settings → Web.".into())
        })?;
    let url = format!(
        "{}/search?{}",
        base.trim_end_matches('/'),
        encode(&[("q", query), ("format", "json")])
    );
    let response = client.get(url).send().await.map_err(network(provider))?;
    if response.status() == StatusCode::FORBIDDEN {
        return Err(WebError::Setup(
            "This SearXNG server doesn't give results as JSON. Add `json` to `search.formats` in its \
             settings.yml."
                .into(),
        ));
    }
    Ok(hits_from(
        &json_of(provider, response).await?["results"],
        "content",
    ))
}

/// Hits from a JSON list of `{ title, url, <snippet> }`.
fn hits_from(results: &Value, snippet: &str) -> Vec<SearchHit> {
    results
        .as_array()
        .map(|results| {
            results
                .iter()
                .map(|result| SearchHit {
                    title: result["title"].as_str().unwrap_or_default().to_owned(),
                    url: result["url"].as_str().unwrap_or_default().to_owned(),
                    snippet: result[snippet].as_str().unwrap_or_default().to_owned(),
                })
                .collect()
        })
        .unwrap_or_default()
}

static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").expect("a valid regex"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("a valid regex"));

/// Plain, single-line text (snippets come with `<strong>` tags and entities).
fn plain(text: &str) -> String {
    let text = TAGS.replace_all(text, "");
    let text = text
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ");
    SPACES.replace_all(text.trim(), " ").into_owned()
}

fn shorten(text: String, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", text[..cut].trim_end()),
        None => text,
    }
}

/// Web pages only, each once, tidied, at most `count`.
fn clean(hits: Vec<SearchHit>, count: usize) -> Vec<SearchHit> {
    let mut seen = HashSet::new();
    hits.into_iter()
        .filter(|hit| hit.url.starts_with("https://") || hit.url.starts_with("http://"))
        .filter(|hit| seen.insert(hit.url.trim_end_matches('/').to_owned()))
        .map(|hit| SearchHit {
            title: shorten(plain(&hit.title), 160),
            snippet: shorten(plain(&hit.snippet), SNIPPET_CHARS),
            url: hit.url,
        })
        .take(count)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUCKDUCKGO: &str = r#"<html><body><div id="links" class="results">
<div class="result results_links results_links_deep result--ad">
  <div class="links_main links_deep result__body">
    <h2 class="result__title"><a class="result__a" href="https://duckduckgo.com/y.js?ad_domain=shop.example&amp;u3=x">Buy things</a></h2>
    <a class="result__snippet" href="https://duckduckgo.com/y.js?ad_domain=shop.example">An ad.</a>
  </div>
</div>
<div class="result results_links results_links_deep web-result ">
  <div class="links_main links_deep result__body">
    <h2 class="result__title">
      <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust%2Dlang.org%2Flearn&amp;rut=abc">Learn <b>Rust</b></a>
    </h2>
    <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust%2Dlang.org%2Flearn">Get started with <b>Rust</b> &amp; Cargo.</a>
  </div>
</div>
<div class="result results_links results_links_deep web-result ">
  <div class="links_main links_deep result__body">
    <h2 class="result__title"><a rel="nofollow" class="result__a" href="https://doc.rust-lang.org/book/">The Rust Book</a></h2>
    <a class="result__snippet" href="https://doc.rust-lang.org/book/">The   official
      book.</a>
  </div>
</div>
</div></body></html>"#;

    #[test]
    fn duckduckgo_results_are_read_without_ads_and_redirects() {
        let hits = clean(parse_duckduckgo(DUCKDUCKGO), 10);
        assert_eq!(
            hits,
            vec![
                SearchHit {
                    title: "Learn Rust".into(),
                    url: "https://www.rust-lang.org/learn".into(),
                    snippet: "Get started with Rust & Cargo.".into(),
                },
                SearchHit {
                    title: "The Rust Book".into(),
                    url: "https://doc.rust-lang.org/book/".into(),
                    snippet: "The official book.".into(),
                },
            ]
        );
    }

    #[test]
    fn json_providers_give_the_same_hits() {
        let brave = json!({ "web": { "results": [
            { "title": "Tokio", "url": "https://tokio.rs/", "description": "An <strong>async</strong> runtime" }
        ] } });
        assert_eq!(
            clean(hits_from(&brave["web"]["results"], "description"), 5)[0].snippet,
            "An async runtime"
        );
        let searxng = json!({ "results": [
            { "title": "A", "url": "https://a.example/", "content": "one" },
            { "title": "A again", "url": "https://a.example", "content": "dup" },
            { "title": "Files", "url": "ftp://files.example/", "content": "not a page" },
            { "title": "B", "url": "http://b.example/", "content": "two" }
        ] });
        let hits = clean(hits_from(&searxng["results"], "content"), 5);
        let urls: Vec<&str> = hits.iter().map(|h| h.url.as_str()).collect();
        assert_eq!(urls, ["https://a.example/", "http://b.example/"]);
        assert_eq!(clean(hits_from(&searxng["results"], "content"), 1).len(), 1);
        assert!(hits_from(&json!({}), "content").is_empty());
    }

    #[test]
    fn long_snippets_are_shortened() {
        let long = "word ".repeat(200);
        let hits = clean(
            vec![SearchHit {
                title: "T".into(),
                url: "https://t.example/".into(),
                snippet: long,
            }],
            1,
        );
        assert!(hits[0].snippet.chars().count() <= SNIPPET_CHARS + 1);
        assert!(hits[0].snippet.ends_with('…'));
    }

    #[test]
    fn queries_are_form_encoded() {
        assert_eq!(encode(&[("q", "rust & c++")]), "q=rust+%26+c%2B%2B");
    }
}
