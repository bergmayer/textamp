//! Album Details reads Navidrome's OpenSubsonic getAlbumInfo2 endpoint. Navidrome
//! chooses its metadata agents (e.g. album-nfo-notes followed by apple-music).
//! AudioMuse supplies related music separately and does not supply these notes.
use super::{NavAction, effects};
use crate::app::{AppState, Event, View, state::PlayAllRow};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct Notes {
    generation: u64,
    key: String,
    pub info: AlbumInfo,
}

impl Notes {
    pub fn matches(&self, generation: u64, key: &str) -> bool {
        self.generation == generation && self.key == key
    }
}

pub fn reconcile(state: &mut AppState, events: &mpsc::Sender<Event>) {
    let target = (state.view == View::Browse && !state.downloads.is_offline())
        .then(|| {
            let nav = state.browse_nav()?;
            nav.columns
                .iter()
                .rev()
                .find_map(|column| match &column.play_all_row {
                    Some(PlayAllRow::Album { rating_key, .. }) => Some(rating_key.as_str()),
                    _ => None,
                })
        })
        .flatten();
    let Some((key, session)) = target.zip(state.sources.active.navidrome()) else {
        if state.sources.album_notes.take().is_some() {
            state.sources.nav_tasks.remove("album-notes");
        }
        return;
    };
    if state
        .sources
        .album_notes
        .as_ref()
        .is_some_and(|notes| notes.matches(state.library_generation, key))
    {
        return;
    }
    let Ok(id) = session.id(key) else { return };
    let client = session.client.clone();
    let key = key.to_owned();
    state.sources.album_notes = Some(Notes {
        generation: state.library_generation,
        key: key.clone(),
        info: AlbumInfo::default(),
    });
    let tx = events.clone();
    let slot = "album-notes";
    effects::spawn_scoped(
        state,
        events,
        slot,
        move |generation, request_id| async move {
            let params = [("id", id)];
            let response = client.call("getAlbumInfo2", &params).await?;
            let info = AlbumInfo::from_response(&response["albumInfo"]);
            if info.link.is_none() || !info.review.is_empty() {
                return Ok(vec![NavAction::AlbumNotes { key, info }.into()]);
            }
            // Navidrome can publish a catalog link before a fallback agent supplies notes.
            let progress = NavAction::Progress {
                generation,
                request_id,
                slot,
                actions: vec![
                    NavAction::AlbumNotes {
                        key: key.clone(),
                        info,
                    }
                    .into(),
                ],
            };
            if tx.send(Event::Effect(progress.into())).await.is_err() {
                return Ok(vec![]);
            }
            for delay in [5, 15] {
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                let response = client.call("getAlbumInfo2", &params).await?;
                let info = AlbumInfo::from_response(&response["albumInfo"]);
                if !info.review.is_empty() {
                    return Ok(vec![NavAction::AlbumNotes { key, info }.into()]);
                }
            }
            Ok(vec![])
        },
    );
}

#[derive(Debug, Clone, Default)]
pub struct AlbumInfo {
    pub review: Vec<Passage>,
    link: Option<reqwest::Url>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Passage {
    pub text: String,
    pub italic: bool,
}

impl AlbumInfo {
    /// Apple Music discovery is available from the command palette. A catalog
    /// match can accompany another provider’s notes and does not credit the review.
    pub fn display_link(&self) -> Option<&reqwest::Url> {
        self.link
            .as_ref()
            .filter(|url| !matches!(url.domain(), Some("music.apple.com" | "itunes.apple.com")))
    }

    fn from_response(info: &serde_json::Value) -> Self {
        let link = info["lastFmUrl"]
            .as_str()
            .and_then(|value| reqwest::Url::parse(value).ok())
            .filter(|url| {
                matches!(url.scheme(), "https" | "http")
                    && url.has_host()
                    && url.username().is_empty()
                    && url.password().is_none()
            });
        Self {
            link,
            review: passages(info["notes"].as_str().unwrap_or_default()),
        }
    }
}

// Emit only literal text and emphasis. Attributes, embedded resources and HTML
// links never reach a renderer. The provider URL is a separate API field.
fn passages(html: &str) -> Vec<Passage> {
    use scraper::{ElementRef, Html};
    let document = Html::parse_fragment(html);
    let mut runs: Vec<Passage> = Vec::new();
    let mut paragraph = false;
    for edge in document.root_element().traverse() {
        use ego_tree::iter::Edge;
        let (node, opening) = match edge {
            Edge::Open(node) => (node, true),
            Edge::Close(node) => (node, false),
        };
        if node.ancestors().filter_map(ElementRef::wrap).any(|el| {
            matches!(
                el.value().name(),
                "script" | "style" | "template" | "iframe" | "svg" | "noscript"
            )
        }) {
            continue;
        }
        let text = match ElementRef::wrap(node).map(|e| e.value().name()) {
            Some("p" | "div" | "li") => {
                paragraph = true;
                continue;
            }
            Some("br") if opening => "\n",
            _ if opening && node.value().is_text() => node.value().as_text().unwrap(),
            _ => continue,
        };
        let text = crate::util::sanitize_display_text(text);
        if text.is_empty() || (paragraph && text.trim().is_empty()) {
            continue;
        }
        if paragraph && let Some(last) = runs.last_mut() {
            last.text.push_str("\n\n");
        }
        paragraph = false;
        let italic = node
            .ancestors()
            .filter_map(ElementRef::wrap)
            .any(|el| matches!(el.value().name(), "em" | "i"));
        if let Some(last) = runs.last_mut()
            && last.italic == italic
        {
            last.text.push_str(&text);
        } else {
            runs.push(Passage {
                text: text.into_owned(),
                italic,
            });
        }
    }
    if runs.iter().all(|r| r.text.trim().is_empty()) {
        runs.clear();
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn album_review_preserves_only_text_paragraphs_and_emphasis() {
        let info = AlbumInfo::from_response(&json!({"notes":
            "<p>One &amp; <em onclick='bad()'>two</em><br>three</p><script>bad()</script><p>Source: Apple Music</p>",
            "lastFmUrl": "javascript:bad()"}));
        let runs: Vec<_> = info
            .review
            .iter()
            .map(|r| (r.text.as_str(), r.italic))
            .collect();
        assert_eq!(
            runs,
            [
                ("One & ", false),
                ("two", true),
                ("\nthree\n\nSource: Apple Music", false)
            ]
        );
        assert!(info.link.is_none());
    }

    #[test]
    fn album_review_and_url_are_independent() {
        let url = "https://music.apple.com/us/album/heartbreaker/6818615462";
        for notes in [None, Some(""), Some("<p> </p>")] {
            let info = AlbumInfo::from_response(&json!({"notes":notes,"lastFmUrl":url}));
            assert!(info.review.is_empty());
            assert!(info.display_link().is_none());
        }
        let info = AlbumInfo::from_response(&json!({"notes":"<p>Review</p><p>Credit</p>"}));
        assert_eq!(info.review[0].text, "Review\n\nCredit");
        assert!(info.link.is_none());
    }
}
