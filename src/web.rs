//! Read-only web tools: DuckDuckGo search and text-only HTTPS fetch.
//!
//! No browser, no cookies, no downloads. Every call is approved by the agent.

use std::net::IpAddr;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use scraper::{ElementRef, Html, Node, Selector};
use url::Url;

const MAX_BODY: usize = 2_000_000;
const MAX_TEXT: usize = 20_000;
const MAX_RESULTS: usize = 8;
const USER_AGENT: &str = "LocalHarness/0.1 (read-only web tools)";

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .user_agent(USER_AGENT)
        .build()
        .context("building HTTP client")
}

fn blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
                || ip.octets()[0] == 0
                || ip.octets()[0] >= 224
                || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
                || (ip.octets()[0] == 198 && matches!(ip.octets()[1], 18 | 19))
                || (ip.octets()[0] == 192 && ip.octets()[1] == 0 && ip.octets()[2] == 0)
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
                || ip.to_ipv4_mapped().map(|ip| blocked_ip(IpAddr::V4(ip))).unwrap_or(false)
        }
    }
}

async fn validate_url(url: &Url) -> Result<()> {
    if url.scheme() != "https" {
        bail!("only https URLs are allowed");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("URLs with credentials are not allowed");
    }
    let host = url.host_str().context("URL has no host")?;
    let lower = host.to_ascii_lowercase();
    if lower == "localhost"
        || lower.ends_with(".localhost")
        || lower.ends_with(".local")
        || lower == "metadata.google.internal"
    {
        bail!("local hostnames are not allowed");
    }
    let port = url.port_or_known_default().context("URL has no port")?;
    if port != 443 {
        bail!("only port 443 is allowed");
    }
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .with_context(|| format!("resolving {host}"))?;
    let mut found = false;
    for address in addresses {
        found = true;
        if blocked_ip(address.ip()) {
            bail!("URL resolves to a private or reserved address");
        }
    }
    if !found {
        bail!("host did not resolve");
    }
    Ok(())
}

async fn read_limited(response: reqwest::Response) -> Result<Vec<u8>> {
    if let Some(length) = response.content_length() {
        if length > MAX_BODY as u64 {
            bail!("response is too large ({length} bytes)");
        }
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading response body")?;
        if body.len() + chunk.len() > MAX_BODY {
            bail!("response exceeded {MAX_BODY} bytes");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn is_text_type(content_type: &str) -> bool {
    let content_type = content_type.to_ascii_lowercase();
    content_type.starts_with("text/")
        || content_type.starts_with("application/json")
        || content_type.starts_with("application/xml")
        || content_type.starts_with("application/xhtml")
}

fn collect_text(node: ElementRef<'_>, output: &mut String) {
    for child in node.children() {
        match child.value() {
            Node::Text(text) => output.push_str(text),
            Node::Element(element) => {
                let name = element.name();
                if matches!(name, "script" | "style" | "noscript" | "svg" | "template" | "head") {
                    continue;
                }
                output.push(' ');
                if let Some(child) = ElementRef::wrap(child) {
                    collect_text(child, output);
                }
                output.push(' ');
            }
            _ => {}
        }
    }
}

fn html_to_text(html: &str) -> (String, String) {
    let document = Html::parse_document(html);
    let title = document
        .select(&Selector::parse("title").expect("title selector"))
        .next()
        .map(|title| title.text().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let mut raw = String::new();
    collect_text(document.root_element(), &mut raw);
    let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    (title.trim().to_string(), text)
}

fn truncate(text: String) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(MAX_TEXT).collect();
    if chars.next().is_some() {
        out.push_str("\n… (truncated)");
    }
    out
}

fn decode_result_url(href: &str) -> String {
    if let Ok(url) = Url::parse(href) {
        for (key, value) in url.query_pairs() {
            if key == "uddg" {
                return value.into_owned();
            }
        }
    }
    if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href.to_string()
    }
}

pub async fn search(query: &str) -> Result<String> {
    let query = query.trim();
    if query.is_empty() {
        bail!("search query is empty");
    }
    let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
    let url = Url::parse(&format!("https://html.duckduckgo.com/html/?q={encoded}"))?;
    validate_url(&url).await?;
    let response = client()?.get(url).send().await.context("contacting DuckDuckGo")?;
    if !response.status().is_success() {
        bail!("DuckDuckGo returned HTTP {}", response.status());
    }
    let body = read_limited(response).await?;
    let html = String::from_utf8_lossy(&body);
    let document = Html::parse_document(&html);
    let anchors = Selector::parse("a.result__a").expect("result selector");
    let snippets = Selector::parse(".result__snippet").expect("snippet selector");
    let mut output = String::new();
    for (index, anchor) in document.select(&anchors).take(MAX_RESULTS).enumerate() {
        let title = anchor.text().collect::<Vec<_>>().join(" ");
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        if title.is_empty() {
            continue;
        }
        let href = anchor.value().attr("href").unwrap_or_default();
        let url = decode_result_url(href);
        let snippet = document
            .select(&snippets)
            .nth(index)
            .map(|snippet| snippet.text().collect::<Vec<_>>().join(" "))
            .unwrap_or_default();
        let snippet = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
        output.push_str(&format!("{}. {}\n   {}\n", index + 1, title, url));
        if !snippet.is_empty() {
            output.push_str(&format!("   {}\n", snippet));
        }
        output.push('\n');
    }
    if output.trim().is_empty() {
        bail!("no search results found");
    }
    Ok(format!("Read-only web search results for \"{query}\":\n\n{output}"))
}

pub async fn fetch(url: &str) -> Result<String> {
    let mut url = Url::parse(url.trim()).context("invalid URL")?;
    for _ in 0..4 {
        validate_url(&url).await?;
        let response = client()?.get(url.clone()).send().await.context("fetching URL")?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .context("redirect without a Location header")?;
            url = url.join(location).context("invalid redirect URL")?;
            continue;
        }
        if !response.status().is_success() {
            bail!("HTTP {}", response.status());
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("text/plain")
            .to_string();
        if !is_text_type(&content_type) {
            bail!("unsupported content type: {content_type}");
        }
        let body = read_limited(response).await?;
        let text = String::from_utf8_lossy(&body);
        let (title, text) = if content_type.to_ascii_lowercase().contains("html") {
            html_to_text(&text)
        } else {
            (String::new(), text.split_whitespace().collect::<Vec<_>>().join(" "))
        };
        let mut output = format!("URL: {url}\n");
        if !title.is_empty() {
            output.push_str(&format!("TITLE: {title}\n"));
        }
        output.push_str("\n");
        output.push_str(&truncate(text));
        return Ok(output);
    }
    bail!("too many redirects");
}
