//! Provider-first biography loading, owned by the popup that requested it.
use crate::app::event::UiEvent;
use crate::app::state::ArtistBioPopup;
use crate::app::tasks::{self, TaskLease};
use crate::app::{AppState, Event};

use crate::services::biography::{self, BioImage, Biography};
use tokio::sync::mpsc;

pub fn show(state: &mut AppState, tx: &mpsc::Sender<Event>, key: String, name: String) {
    state.artist_bio_request_id = state.artist_bio_request_id.wrapping_add(1);
    let request_id = state.artist_bio_request_id;
    let generation = state.library_generation;
    state.popups.close_all();

    let navidrome = state.sources.active.navidrome().cloned();
    let tx = tx.clone();
    let artist = name.clone();
    let task = tasks::spawn(async move {
        let result = if let Some(session) = navidrome {
            load_navidrome(&session, &key, &artist).await
        } else {
            biography::wikipedia(&artist)
                .await
                .map_err(|e| unavailable_message(None, &format!("{e:#}")))
        };
        let _ = tx
            .send(
                UiEvent::ArtistBioLoaded {
                    generation,
                    request_id,
                    result,
                }
                .into(),
            )
            .await;
    });
    state.popups.artist_bio = Some(ArtistBioPopup {
        artist_name: name,
        document: Biography::default(),
        scroll: 0,
        google_focused: false,
        loading: true,
        image_index: 0,
        task: Some(TaskLease::new(&task)),
    });
}

async fn load_navidrome(
    session: &crate::app::sources::navidrome::Session,
    key: &str,
    name: &str,
) -> Result<Biography, String> {
    let provider = async {
        let id = session.id(key)?;
        let params = [("id", id), ("count", "0".into())];
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            session.client.call("getArtistInfo2", &params),
        )
        .await??;
        let info = &response["artistInfo2"];
        let text = biography::fragment(info["biography"].as_str().unwrap_or_default());
        let mut document = Biography {
            text,
            ..Default::default()
        };
        if let Some(url) = info["lastFmUrl"]
            .as_str()
            .and_then(|s| reqwest::Url::parse(s).ok())
            .filter(|u| matches!(u.scheme(), "https" | "http"))
        {
            document.source_url = Some(url.to_string());
        }
        if !document.text.trim().is_empty() {
            if let Some(url) = info["largeImageUrl"]
                .as_str()
                .and_then(|s| reqwest::Url::parse(s).ok())
                .filter(|u| {
                    matches!(u.scheme(), "https" | "http")
                        && u.username().is_empty()
                        && u.password().is_none()
                })
            {
                // This client has no default Authorization header; never append server credentials to agent URLs.
                match tokio::time::timeout(
                    std::time::Duration::from_secs(3),
                    session.client.bytes(url.clone(), 8 * 1024 * 1024),
                )
                .await
                {
                    Ok(Ok(data)) => document.images.push(BioImage {
                        key: url.to_string(),
                        data,
                        caption: "Artist image supplied by Navidrome".into(),
                    }),
                    Ok(Err(error)) => tracing::warn!("Navidrome biography image: {error}"),
                    Err(_) => tracing::warn!("Navidrome biography image timed out"),
                }
            }
        }
        Ok::<_, anyhow::Error>(document)
    };
    let provider_issue =
        match tokio::time::timeout(std::time::Duration::from_secs(12), provider).await {
            Ok(Ok(document)) if !document.text.trim().is_empty() => return Ok(document),
            Ok(Err(error)) => format!("{error:#}"),
            Err(_) => "Biography lookup timed out".into(),
            _ => "No biography supplied".into(),
        };
    tracing::debug!("Navidrome biography unavailable: {provider_issue}");
    biography::wikipedia(name)
        .await
        .map_err(|e| unavailable_message(Some(&provider_issue), &format!("{e:#}")))
}

fn unavailable_message(navidrome: Option<&str>, wikipedia: &str) -> String {
    match navidrome {
        Some(issue) => format!("No biography available from Navidrome or Wikipedia.\n\nNavidrome: {issue}\nWikipedia: {wikipedia}"),
        None => format!("No biography available from Wikipedia.\n\nWikipedia: {wikipedia}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_biography_names_attempted_sources_and_preserves_failures() {
        let message =
            unavailable_message(Some("Authentication failed"), "No matching musical artist");
        assert!(message.starts_with("No biography available from Navidrome or Wikipedia."));
        assert!(message.contains("Navidrome: Authentication failed"));
        assert!(message.contains("Wikipedia: No matching musical artist"));
        let local = unavailable_message(None, "Lookup timed out");
        assert!(!local.contains("Navidrome"));
        assert!(local.contains("Wikipedia: Lookup timed out"));
    }
}
