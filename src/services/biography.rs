//! On-demand Wikipedia biographies, resolved through musical-artist identities.
//! No library credentials, paths, or playback history are sent to Wikimedia.
use anyhow::{bail, Context, Result};
use scraper::{ElementRef, Html, Selector};
use serde_json::Value;
use std::time::Duration;

const WIKIDATA: &str = "https://www.wikidata.org/w/api.php";
const WIKIPEDIA: &str = "https://en.wikipedia.org/w/api.php";
const MAX_IMAGES: usize = 8;

/// Explicit user-initiated search; sends only the displayed artist name.
pub fn google_search_url(artist: &str) -> String {
    let query = format!("{} music artist biography", artist.trim());
    format!(
        "https://www.google.com/search?q={}",
        urlencoding::encode(&query)
    )
}

#[derive(Debug, Clone, Default)]
pub struct Biography {
    pub text: String,
    pub source_url: Option<String>,
    pub images: Vec<BioImage>,
}

#[derive(Debug, Clone)]
pub struct BioImage {
    pub key: String,
    pub data: Vec<u8>,
    pub caption: String,
}

/// One bounded operation; cancellation drops HTTP requests with the popup.
pub async fn wikipedia(artist: &str) -> Result<Biography> {
    let client = client()?;
    let (mut biography, photos) =
        tokio::time::timeout(Duration::from_secs(30), lookup(&client, artist))
            .await
            .context("Wikipedia lookup timed out")??;
    // Separate image budget: an image outage cannot discard loaded prose.
    biography.images = tokio::time::timeout(
        Duration::from_secs(10),
        load_images(&client, &photos, &mut biography.text),
    )
    .await
    .unwrap_or_else(|_| {
        biography
            .text
            .push_str("\n\nArticle images timed out; use B to view the source.");
        Vec::new()
    });
    Ok(biography)
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("Textamp/1.0 (music artist biographies; https://github.com/bergmayer/textamp)")
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

async fn bytes(request: reqwest::RequestBuilder, limit: usize) -> Result<Vec<u8>> {
    let mut response = request.send().await.map_err(reqwest::Error::without_url)?;
    if !response.status().is_success() {
        bail!("Wikimedia returned HTTP {}", response.status().as_u16());
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        bail!("Wikimedia response exceeds size limit");
    }
    let mut data = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)?
    {
        if data.len().saturating_add(chunk.len()) > limit {
            bail!("Wikimedia response exceeds size limit");
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

async fn api(client: &reqwest::Client, endpoint: &str, params: &[(&str, &str)]) -> Result<Value> {
    let data = bytes(
        client.get(endpoint).query(params).query(&[
            ("format", "json"),
            ("formatversion", "2"),
            ("maxlag", "5"),
        ]),
        4 * 1024 * 1024,
    )
    .await?;
    let value: Value = serde_json::from_slice(&data).context("Invalid Wikimedia response")?;
    if value.get("error").is_some() {
        bail!("Wikimedia could not complete this request; try again later");
    }
    Ok(value)
}

fn normalized(name: &str) -> String {
    // Preserve punctuation: AC/DC, !!! and similarly named artists are distinct.
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn claims<'a>(entity: &'a Value, property: &str) -> impl Iterator<Item = &'a Value> {
    entity["claims"][property]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|claim| claim["rank"] != "deprecated")
        .filter_map(|claim| claim.pointer("/mainsnak/datavalue/value"))
}

fn musical_artist(entity: &Value) -> bool {
    if claims(entity, "P31").any(|v| v["id"] == "Q4167410") {
        return false;
    }
    // MusicBrainz artist ID is an established identity, not a guessed description.
    if claims(entity, "P434").any(|v| {
        v.as_str()
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
    }) {
        return true;
    }
    // Conservative direct types/occupations for artists without a MusicBrainz ID.
    claims(entity, "P31")
        .chain(claims(entity, "P106"))
        .any(|v| {
            matches!(
                v["id"].as_str(),
                Some(
                    "Q215380"
                        | "Q2088357"
                        | "Q42998"
                        | "Q639669"
                        | "Q177220"
                        | "Q36834"
                        | "Q753110"
                        | "Q488205"
                )
            )
        })
}

fn exact_name(entity: &Value, artist: &str) -> bool {
    ["en", "mul"].iter().any(|language| {
        entity["labels"][language]["value"]
            .as_str()
            .is_some_and(|name| normalized(name) == artist)
            || entity["aliases"][language]
                .as_array()
                .into_iter()
                .flatten()
                .any(|alias| {
                    alias["value"]
                        .as_str()
                        .is_some_and(|name| normalized(name) == artist)
                })
    })
}

/// Never choose the first search hit or silently rank two same-name musicians.
fn select_artist<'a>(entities: &'a Value, artist: &str) -> Result<(&'a str, &'a str)> {
    let artist = normalized(artist);
    let entities = entities.as_object().context("Missing Wikidata entities")?;
    let mut matches = entities
        .iter()
        .filter(|(_, entity)| exact_name(entity, &artist) && musical_artist(entity))
        .filter_map(|(id, entity)| {
            entity["sitelinks"]["enwiki"]["title"]
                .as_str()
                .map(|title| (id.as_str(), title))
        });
    let first = matches
        .next()
        .context("No confidently matched musical artist on English Wikipedia")?;
    if matches.next().is_some() {
        bail!("Several musical artists share this name; no biography selected");
    }
    Ok(first)
}

async fn lookup(
    client: &reqwest::Client,
    artist: &str,
) -> Result<(Biography, Vec<(String, String)>)> {
    if artist.trim().is_empty() || artist.len() > 300 {
        bail!("Artist name is missing or too long");
    }
    let search = api(
        client,
        WIKIDATA,
        &[
            ("action", "wbsearchentities"),
            ("search", artist),
            ("language", "en"),
            ("limit", "20"),
            ("type", "item"),
        ],
    )
    .await?;
    let ids = search["search"]
        .as_array()
        .context("Missing Wikidata search results")?
        .iter()
        .filter_map(|v| v["id"].as_str())
        .collect::<Vec<_>>()
        .join("|");
    if ids.is_empty() {
        bail!("No matching musical artist on Wikipedia");
    }
    let entities = api(
        client,
        WIKIDATA,
        &[
            ("action", "wbgetentities"),
            ("ids", &ids),
            ("props", "labels|aliases|claims|sitelinks"),
            ("languages", "en|mul"),
            ("sitefilter", "enwiki"),
        ],
    )
    .await?;
    let (id, title) = select_artist(&entities["entities"], artist)?;
    let page = api(
        client,
        WIKIPEDIA,
        &[
            ("action", "query"),
            ("titles", title),
            ("prop", "pageprops"),
            ("redirects", "1"),
        ],
    )
    .await?;
    let page = &page["query"]["pages"][0];
    if page["pageprops"]["wikibase_item"] != id || page["pageprops"].get("disambiguation").is_some()
    {
        bail!("Wikipedia article does not match the verified artist");
    }
    let pageid = page["pageid"]
        .as_u64()
        .context("Wikipedia article unavailable")?
        .to_string();
    let article = api(
        client,
        WIKIPEDIA,
        &[
            ("action", "parse"),
            ("pageid", &pageid),
            ("prop", "text|revid"),
            ("disableeditsection", "1"),
        ],
    )
    .await?;
    let html = article["parse"]["text"]
        .as_str()
        .context("Wikipedia article text unavailable")?;
    let (mut text, photos) = article_content(html);
    if text.is_empty() {
        bail!("Wikipedia article has no readable biography");
    }
    let revision = article["parse"]["revid"]
        .as_u64()
        .context("Wikipedia revision unavailable")?;
    let source_url = format!("https://en.wikipedia.org/w/index.php?oldid={revision}");
    text.push_str(&format!("\n\nSource: Wikipedia contributors — {title}\n{source_url}\nText: CC BY-SA 4.0 — https://creativecommons.org/licenses/by-sa/4.0/\nHTML reformatted for Textamp; navigation, references and tables omitted."));
    Ok((
        Biography {
            text,
            source_url: Some(source_url),
            images: Vec::new(),
        },
        photos,
    ))
}

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("static HTML selector")
}

fn unwanted(element: ElementRef<'_>) -> bool {
    matches!(
        element.value().name(),
        "script" | "style" | "sup" | "table" | "nav"
    ) || element.value().classes().any(|class| {
        matches!(
            class,
            "navbox"
                | "vertical-navbox"
                | "infobox"
                | "sidebar"
                | "hatnote"
                | "toc"
                | "reflist"
                | "reference"
                | "mw-editsection"
                | "metadata"
                | "noprint"
                | "sistersitebox"
        )
    })
}

fn plain(element: ElementRef<'_>) -> String {
    let mut text = String::new();
    for node in element.descendants() {
        if node.ancestors().filter_map(ElementRef::wrap).any(unwanted) {
            continue;
        }
        if let Some(value) = node.value().as_text() {
            text.push_str(value);
        }
        if ElementRef::wrap(node).is_some_and(|e| e.value().name() == "br") {
            text.push(' ');
        }
    }
    crate::util::sanitize_display_text(&text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn fragment(html: &str) -> String {
    plain(Html::parse_fragment(html).root_element())
}

/// Retain article prose/headings/lists and meaningful photos, not site chrome.
fn article_content(html: &str) -> (String, Vec<(String, String)>) {
    let document = Html::parse_fragment(html);
    let mut paragraphs = Vec::new();
    let mut length = 0;
    for element in document.select(&selector("p, h2, h3, h4, li")) {
        if element
            .ancestors()
            .filter_map(ElementRef::wrap)
            .any(unwanted)
        {
            continue;
        }
        if element
            .ancestors()
            .skip(1)
            .filter_map(ElementRef::wrap)
            .any(|e| matches!(e.value().name(), "p" | "li"))
        {
            continue;
        }
        let text = plain(element);
        if matches!(element.value().name(), "h2" | "h3")
            && matches!(
                text.as_str(),
                "References"
                    | "Notes"
                    | "External links"
                    | "Further reading"
                    | "See also"
                    | "Sources"
            )
        {
            break;
        }
        if text.is_empty() {
            continue;
        }
        length += text.len();
        if length > 200_000 {
            paragraphs.push("[Article shortened; see source for full text.]".into());
            break;
        }
        paragraphs.push(if element.value().name() == "li" {
            format!("• {text}")
        } else {
            text
        });
    }
    let mut photos = Vec::new();
    for img in document.select(&selector("img")) {
        let width = img
            .value()
            .attr("width")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        let height = img
            .value()
            .attr("height")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        if width < 120 || height < 60 {
            continue;
        }
        let Some(link) = img
            .ancestors()
            .filter_map(ElementRef::wrap)
            .find(|e| e.value().name() == "a")
        else {
            continue;
        };
        let href = link.value().attr("href").unwrap_or("");
        let Some(file) = href
            .strip_prefix("/wiki/File:")
            .or_else(|| href.strip_prefix("./File:"))
        else {
            continue;
        };
        let Ok(file) = urlencoding::decode(file) else {
            continue;
        };
        let file = format!("File:{}", file.replace('_', " "));
        if photos.iter().any(|(name, _)| name == &file) {
            continue;
        }
        let caption = img
            .ancestors()
            .filter_map(ElementRef::wrap)
            .find(|e| e.value().name() == "figure" || e.value().classes().any(|c| c == "thumb"))
            .and_then(|e| e.select(&selector("figcaption, .thumbcaption")).next())
            .map(plain)
            .unwrap_or_else(|| img.value().attr("alt").unwrap_or("").into());
        photos.push((
            file,
            crate::util::sanitize_display_text(&caption).into_owned(),
        ));
        if photos.len() == MAX_IMAGES {
            break;
        }
    }
    (paragraphs.join("\n\n"), photos)
}

fn image_url(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("upload.wikimedia.org")
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
    })
}

async fn load_images(
    client: &reqwest::Client,
    photos: &[(String, String)],
    text: &mut String,
) -> Vec<BioImage> {
    let mut images = Vec::new();
    if photos.is_empty() {
        return images;
    }
    let titles = photos
        .iter()
        .map(|(title, _)| title.as_str())
        .collect::<Vec<_>>()
        .join("|");
    let result = api(
        client,
        WIKIPEDIA,
        &[
            ("action", "query"),
            ("titles", &titles),
            ("prop", "imageinfo"),
            ("iiprop", "url|extmetadata"),
            ("iiurlwidth", "600"),
        ],
    )
    .await;
    let Ok(value) = result else {
        text.push_str("\n\nArticle images could not be loaded.");
        return images;
    };
    for (title, caption) in photos {
        let info = value["query"]["pages"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| p["title"] == *title)
            .map(|p| &p["imageinfo"][0]);
        let Some(info) = info else {
            continue;
        };
        let url = info["thumburl"].as_str().unwrap_or("");
        if !image_url(url) {
            continue;
        }
        // Keep only images with usable licensing metadata; no guessing at rights.
        let metadata = &info["extmetadata"];
        let license = fragment(metadata["LicenseShortName"]["value"].as_str().unwrap_or(""));
        let author = fragment(metadata["Artist"]["value"].as_str().unwrap_or(""));
        if !(license.starts_with("CC") || license == "Public domain" || license.starts_with("GFDL"))
        {
            continue;
        }
        let license_url = fragment(metadata["LicenseUrl"]["value"].as_str().unwrap_or(""));
        let credit = fragment(metadata["Credit"]["value"].as_str().unwrap_or(""));
        match bytes(client.get(url), 4 * 1024 * 1024).await {
            Ok(data) => {
                text.push_str(&format!(
                    "\n\nImage {}: {}\n{} — {}\n{}\n{}\nhttps://en.wikipedia.org/wiki/{}",
                    images.len() + 1,
                    caption,
                    author,
                    license,
                    license_url,
                    credit,
                    urlencoding::encode(title)
                ));
                images.push(BioImage {
                    key: url.into(),
                    data,
                    caption: caption.clone(),
                });
            }
            Err(_) => text.push_str(&format!("\n\nImage unavailable: {title}")),
        }
    }
    images
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn artist(name: &str, title: &str) -> Value {
        json!({"labels":{"en":{"value":name}},"claims":{"P434":[{"mainsnak":{"datavalue":{"value":"550e8400-e29b-41d4-a716-446655440000"}}}]},"sitelinks":{"enwiki":{"title":title}}})
    }
    #[test]
    fn strict_matching_selects_music_not_tools_and_rejects_ambiguity() {
        let mut values = json!({"Q1": {"labels":{"en":{"value":"Tool"}},"sitelinks":{"enwiki":{"title":"Tool"}}}, "Q2": artist("Tool", "Tool (band)")});
        assert_eq!(
            select_artist(&values, " TOOL ").unwrap(),
            ("Q2", "Tool (band)")
        );
        assert!(select_artist(&values, "Tools").is_err());
        values["Q3"] = artist("Tool", "Another Tool");
        assert!(select_artist(&values, "Tool")
            .unwrap_err()
            .to_string()
            .contains("Several"));
    }
    #[test]
    fn aliases_work_but_disambiguation_and_deprecated_claims_do_not() {
        let mut value = artist("Björk", "Björk");
        value["aliases"] = json!({"en":[{"value":"Bjork"}]});
        assert!(select_artist(&json!({"Q1":value.clone()}), "Bjork").is_ok());
        value["claims"]["P434"][0]["rank"] = json!("deprecated");
        assert!(!musical_artist(&value));
        value["claims"]["P434"][0]["rank"] = json!("normal");
        value["claims"]["P31"] = json!([{"mainsnak":{"datavalue":{"value":{"id":"Q4167410"}}}}]);
        assert!(!musical_artist(&value));
        assert_ne!(normalized("AC/DC"), normalized("ACDC"));
    }
    #[test]
    fn html_conversion_keeps_prose_sections_and_photos_without_chrome() {
        let (text, photos) = article_content(
            r#"<div class="hatnote">Not hand tools</div><table class="infobox"><tr><td>Chrome<img width="200" height="150" src="x"></td></tr></table><p>Tool is a <b>band</b> &amp; musicians.<sup class="reference">[1]</sup><script>bad()</script></p><h2>History<span class="mw-editsection">edit</span></h2><p>First<br>second.</p><figure><a href="/wiki/File:Band_photo.jpg"><img width="220" height="160"></a><figcaption>The band</figcaption></figure><ul><li>One</li><li>Two</li></ul><h2>References</h2><p>Citation noise</p>"#,
        );
        assert!(text.contains("Tool is a band & musicians."));
        assert!(text.contains("History\n\nFirst second."));
        assert!(text.contains("• One\n\n• Two"));
        for noise in [
            "hand tools",
            "Chrome",
            "[1]",
            "bad()",
            "edit",
            "Citation noise",
        ] {
            assert!(!text.contains(noise), "{noise}");
        }
        assert_eq!(photos, [("File:Band photo.jpg".into(), "The band".into())]);
        assert!(image_url(
            "https://upload.wikimedia.org/wikipedia/commons/photo.jpg"
        ));
        assert!(!image_url(
            "https://upload.wikimedia.org.evil.test/photo.jpg"
        ));
        assert!(!image_url("http://127.0.0.1/private"));
    }

    #[tokio::test]
    #[ignore = "Live Wikimedia requests; run explicitly"]
    async fn live_tool_resolves_band_with_article_and_images() {
        let bio = wikipedia("Tool").await.unwrap();
        assert!(
            bio.text.contains("Tool is an American"),
            "{}",
            &bio.text[..bio.text.len().min(120)]
        );
        assert!(bio.text.contains("Tool (band)"));
        assert!(
            !bio.images.is_empty(),
            "No images loaded; {}",
            bio.text.lines().last().unwrap_or_default()
        );
        println!(
            "Verified Tool (band): {} text bytes, {} images, {}",
            bio.text.len(),
            bio.images.len(),
            bio.source_url.unwrap()
        );
    }

    #[tokio::test]
    async fn http_errors_and_oversized_responses_are_not_successes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for reply in [
            "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 99999\r\n\r\n",
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 1024];
                assert!(socket.read(&mut request).await.unwrap() > 0);
                socket.write_all(reply.as_bytes()).await.unwrap();
            });
            assert!(
                bytes(client().unwrap().get(format!("http://{address}")), 10)
                    .await
                    .is_err()
            );
            server.await.unwrap();
        }
    }
}
