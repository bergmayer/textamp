//! Navidrome-first biography loading, owned by the popup that requested it.
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
    let mbid = state
        .library
        .artists
        .iter()
        .chain(&state.library.track_artists)
        .find(|artist| artist.rating_key == key)
        .and_then(|artist| artist.music_brainz_id.clone());
    let tx = tx.clone();
    let artist = name.clone();
    let task = tasks::spawn(async move {
        let result = if let Some(session) = navidrome {
            load_navidrome(&session, &key, &artist, mbid).await
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
    cached_mbid: Option<String>,
) -> Result<Biography, String> {
    load_navidrome_with(session, key, cached_mbid, |mbid| async move {
        biography::wikipedia_for_artist(name, mbid.as_deref()).await
    })
    .await
}

async fn load_navidrome_with<F, Fut>(
    session: &crate::app::sources::navidrome::Session,
    key: &str,
    mbid: Option<String>,
    wikipedia: F,
) -> Result<Biography, String>
where
    F: FnOnce(Option<String>) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<Biography>>,
{
    let mut fallback_mbid = mbid
        .as_deref()
        .and_then(biography::valid_mbid)
        .map(|id| id.to_string());
    let provider = async {
        let id = session.id(key)?;
        let params = [("id", id), ("count", "0".into())];
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            session.client.call("getArtistInfo2", &params),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Navidrome artist-info timed out after 8 seconds"))??;
        let info = &response["artistInfo2"];
        let expected = fallback_mbid.as_deref().and_then(biography::valid_mbid);
        let supplied = info["musicBrainzId"]
            .as_str()
            .and_then(biography::valid_mbid);
        if expected.is_some() && supplied.is_some() && supplied != expected {
            anyhow::bail!("Server biography conflicts with the requested MusicBrainz identity");
        }
        if expected.is_none() {
            fallback_mbid = supplied.map(|id| id.to_string());
        }
        let text = biography::fragment(info["biography"].as_str().unwrap_or_default());
        anyhow::ensure!(!text.trim().is_empty(), "No biography supplied");
        let mut document = Biography {
            text,
            ..Default::default()
        };
        if let Some(url) = info["lastFmUrl"]
            .as_str()
            .and_then(|s| reqwest::Url::parse(s).ok())
            .filter(|u| {
                matches!(u.scheme(), "https" | "http")
                    && u.username().is_empty()
                    && u.password().is_none()
            })
        {
            document.source_url = Some(url.to_string());
        }
        if !document.text.trim().is_empty() {
            document
                .text
                .push_str("\n\nSource: biography supplied by Navidrome");
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
    let provider_issue = match provider.await {
        Ok(document) if !document.text.trim().is_empty() => return Ok(document),
        Err(error) => format!("{error:#}"),
        _ => "No biography supplied".into(),
    };
    tracing::debug!("Navidrome biography unavailable: {provider_issue}");
    wikipedia(fallback_mbid)
        .await
        .map_err(|error| unavailable_message(Some(&provider_issue), &format!("{error:#}")))
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
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    async fn server(
        info: serde_json::Value,
        status: u16,
    ) -> (
        crate::app::sources::navidrome::Session,
        Arc<AtomicUsize>,
        tokio::task::JoinHandle<()>,
    ) {
        delayed_server(info, status, std::time::Duration::ZERO).await
    }

    async fn delayed_server(
        mut info: serde_json::Value,
        status: u16,
        delay: std::time::Duration,
    ) -> (
        crate::app::sources::navidrome::Session,
        Arc<AtomicUsize>,
        tokio::task::JoinHandle<()>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        if info["largeImageUrl"] == "fixture-image" {
            info["largeImageUrl"] = json!(format!("http://{address}/image"));
        }
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let n = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..n]);
                let image = request.contains("/image ");
                assert!(image || request.contains("/rest/getArtistInfo2?"));
                count.fetch_add(1, Ordering::SeqCst);
                if !image {
                    tokio::time::sleep(delay).await;
                }
                let status = if image { 503 } else { status };
                let body =
                    json!({"subsonic-response":{"status":"ok","artistInfo2":info}}).to_string();
                socket.write_all(format!("HTTP/1.1 {status} Result\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        });
        let source = crate::navidrome::Source {
            id: "fixture".into(),
            name: "Fixture".into(),
            url: format!("http://{address}"),
            username: "fixture".into(),
            libraries: vec![],
        };
        let client = crate::navidrome::Client::new(
            &source,
            crate::util::SecretString::from("fixture"),
            None,
        )
        .unwrap();
        (
            crate::app::sources::navidrome::Session {
                source,
                client,
                extensions: Default::default(),
            },
            requests,
            task,
        )
    }

    #[tokio::test]
    async fn navidrome_success_never_calls_wikipedia() {
        let (session, requests, task) =
            server(json!({"biography":"An <b>artist</b> biography."}), 200).await;
        let document = load_navidrome_with(&session, &session.key("artist"), None, |_| async {
            panic!("A supplied server biography must take priority over Wikipedia");
        })
        .await
        .unwrap();
        assert!(document.text.starts_with("An artist biography."));
        assert!(document.text.contains("supplied by Navidrome"));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        task.abort();
    }

    #[tokio::test]
    async fn missing_or_failed_server_falls_back_to_wikipedia() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        for (info, status, expected_mbid) in [
            (json!({"biography":"  ","musicBrainzId":id}), 200, Some(id)),
            (json!({}), 200, None),
            (json!({}), 503, None),
        ] {
            let (session, requests, task) = server(info, status).await;
            let document = load_navidrome_with(&session, &session.key("artist"), None, |mbid| {
                assert_eq!(mbid.as_deref(), expected_mbid);
                std::future::ready(Ok(Biography {
                    text: "Wikipedia prose".into(),
                    ..Default::default()
                }))
            })
            .await
            .unwrap();
            assert_eq!(document.text, "Wikipedia prose");
            assert_eq!(requests.load(Ordering::SeqCst), 1);
            task.abort();
        }
    }

    #[tokio::test]
    async fn both_failures_remain_explicit_and_never_become_empty_success() {
        for (info, status, expected_issue) in [
            (json!({"biography":" <br> "}), 200, "No biography supplied"),
            (json!({}), 503, "HTTP 503"),
        ] {
            let (session, _, task) = server(info, status).await;
            let error = load_navidrome_with(&session, &session.key("artist"), None, |_| {
                std::future::ready(Err(anyhow::anyhow!("Wikipedia unavailable")))
            })
            .await
            .unwrap_err();
            assert!(error.contains(expected_issue));
            assert!(error.contains("Wikipedia unavailable"));
            task.abort();
        }
    }

    #[tokio::test]
    async fn conflicting_server_identity_falls_back_with_the_cached_artist_id() {
        let expected = "550e8400-e29b-41d4-a716-446655440000";
        let other = "1c935e13-c373-4e41-a84e-8e9921f8fe7d";
        let (session, _, task) = server(
            json!({"biography":"Wrong namesake","musicBrainzId":other}),
            200,
        )
        .await;
        let error = load_navidrome_with(
            &session,
            &session.key("artist"),
            Some(expected.into()),
            |mbid| {
                assert_eq!(mbid.as_deref(), Some(expected));
                std::future::ready(Err(biography::AmbiguousArtist.into()))
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("conflicts"));
        assert!(error.contains("Several musical artists"));
        assert!(!error.contains("Wrong namesake"));
        task.abort();
    }

    #[tokio::test]
    async fn stalled_server_reaches_wikipedia_within_the_artist_info_budget() {
        let (session, requests, task) = delayed_server(
            json!({"biography":"Too late"}),
            200,
            std::time::Duration::from_secs(30),
        )
        .await;
        let document = tokio::time::timeout(
            std::time::Duration::from_secs(12),
            load_navidrome_with(&session, &session.key("artist"), None, |_| {
                std::future::ready(Ok(Biography {
                    text: "Fallback prose".into(),
                    ..Default::default()
                }))
            }),
        )
        .await
        .expect("A stalled server must not indefinitely block Wikipedia")
        .unwrap();
        assert_eq!(document.text, "Fallback prose");
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        task.abort();
    }

    #[tokio::test]
    async fn server_image_failure_preserves_loaded_prose_without_wikipedia() {
        let (session, requests, task) = server(
            json!({"biography":"Server prose","largeImageUrl":"fixture-image"}),
            200,
        )
        .await;
        let document = load_navidrome_with(&session, &session.key("artist"), None, |_| async {
            panic!("An image failure must not discard the server biography");
        })
        .await
        .unwrap();
        assert!(document.text.starts_with("Server prose"));
        assert!(document.images.is_empty());
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        task.abort();
    }

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
