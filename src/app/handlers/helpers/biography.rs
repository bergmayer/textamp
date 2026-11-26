//! Artist metadata loading for browse columns and biography readers.
use crate::app::event::UiEvent;
use crate::app::state::ArtistBioPopup;
use crate::app::tasks::{self, TaskLease};
use crate::app::{AppState, Event};

use crate::services::biography::{self, BioImage, Biography};
use tokio::sync::mpsc;

enum Destination {
    Window,
    Column,
}

fn request(
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
    key: &str,
    name: &str,
    destination: Destination,
) -> (u64, TaskLease) {
    let counter = match destination {
        Destination::Window => &mut state.artist_bio_request_id,
        Destination::Column => &mut state.artist_info_request_id,
    };
    *counter = counter.wrapping_add(1);
    let request_id = *counter;
    let generation = state.library_generation;
    let navidrome = state.sources.active.navidrome().cloned();
    let mbid = state
        .library
        .artists
        .iter()
        .chain(&state.library.track_artists)
        .find(|artist| artist.rating_key == key)
        .and_then(|artist| artist.music_brainz_id.clone());
    let tx = tx.clone();
    let key = key.to_owned();
    let name = name.to_owned();
    let task = tasks::spawn(async move {
        let result = if let Some(session) = navidrome {
            load_navidrome(&session, &key, &name, mbid, generation, &tx).await
        } else {
            biography::wikipedia(&name)
                .await
                .map_err(|e| unavailable_message(None, &format!("{e:#}")))
        };
        let event = match destination {
            Destination::Window => UiEvent::ArtistBioLoaded {
                generation,
                request_id,
                result,
            },
            Destination::Column => UiEvent::ArtistInfoLoaded {
                generation,
                request_id,
                artist_key: key,
                result,
            },
        };
        let _ = tx.send(event.into()).await;
    });
    (request_id, TaskLease::new(&task))
}
/// Completed artist lookups for the active library, including misses.
#[derive(Debug, Default)]
pub(crate) struct Cache(std::collections::VecDeque<(String, std::time::Instant, Biography)>);

impl Cache {
    pub(crate) fn get(&self, key: &str) -> Option<&Biography> {
        self.0.iter().find_map(|(artist, fetched, document)| {
            let ttl = if document.has_prose() {
                24 * 60 * 60
            } else {
                60 * 60
            };
            (artist == key && fetched.elapsed() < std::time::Duration::from_secs(ttl))
                .then_some(document)
        })
    }

    pub(crate) fn insert(&mut self, key: &str, document: &Biography) {
        self.0.retain(|(artist, _, _)| artist != key);
        self.0
            .push_front((key.to_owned(), std::time::Instant::now(), document.clone()));
        self.0.truncate(32);
    }

    pub(crate) fn documents_mut(&mut self) -> impl Iterator<Item = &mut Biography> {
        self.0.iter_mut().map(|(_, _, document)| document)
    }

    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }
}

/// Resolve a button and its reader from the same artist document. A pending
/// column or an empty result must not hide prose already loaded for this artist.
pub(crate) fn known<'a>(state: &'a AppState, key: &str) -> Option<&'a Biography> {
    let album = state
        .sources
        .album_artists
        .as_ref()
        .filter(|r| r.generation == state.library_generation && r.artist == key)
        .and_then(|r| r.document.as_ref());
    let headers = state
        .browse_nav()
        .into_iter()
        .flat_map(|nav| &nav.columns)
        .filter_map(|column| column.artist_header.as_ref())
        .filter(|info| info.artist_key == key && (info.task.is_none() || info.document.has_prose()))
        .map(|info| &info.document);
    let mut documents = state
        .artist_bios
        .get(key)
        .into_iter()
        .chain(album)
        .chain(headers);
    documents
        .clone()
        .find(|doc| doc.has_prose())
        .or_else(|| documents.next())
}

pub fn show(state: &mut AppState, tx: &mpsc::Sender<Event>, key: String, name: String) {
    state.popups.close_all();
    let existing = known(state, &key);
    let (document, task) = if let Some(document) = existing {
        (document.clone(), None)
    } else if state.downloads.is_offline() {
        (
            crate::app::downloads::offline_artist(state, &key, &name, None).unwrap_or_else(|| {
                Biography {
                    unavailable: true,
                    text: "Biography unavailable offline".into(),
                    ..Default::default()
                }
            }),
            None,
        )
    } else {
        (
            Biography::default(),
            Some(request(state, tx, &key, &name, Destination::Window).1),
        )
    };
    if task.is_none() {
        state.artist_bio_request_id = state.artist_bio_request_id.wrapping_add(1);
    }
    state.popups.set_artist_bio(Some(ArtistBioPopup {
        artist_key: key,
        artist_name: name,
        document,
        scroll: 0,
        google_focused: false,
        image_index: 0,
        task,
    }));
}
pub fn column_info(
    state: &mut AppState,
    tx: &mpsc::Sender<Event>,
    key: String,
    name: String,
) -> crate::app::state::ArtistInfoDetails {
    let (request_id, task, document) = if let Some(document) = state.artist_bios.get(&key) {
        (0, None, document.clone())
    } else if state.downloads.is_offline() {
        let document = crate::app::downloads::offline_artist(state, &key, &name, None)
            .unwrap_or_else(|| Biography {
                unavailable: true,
                text: "Biography unavailable offline".into(),
                ..Default::default()
            });
        (0, None, document)
    } else {
        let (id, task) = request(state, tx, &key, &name, Destination::Column);
        (id, Some(task), Biography::default())
    };
    if document.cover_art.is_none()
        && !state.artwork.grid_cache.contains_key(&key)
        && let Some(image) = document.images.first()
    {
        state
            .artwork
            .insert_grid_art(key.clone(), image.data.clone());
    }
    crate::app::state::ArtistInfoDetails {
        artist_key: key,
        artist_name: name,
        request_id,
        document,
        image_index: 0,
        task,
    }
}
pub fn show_info(state: &mut AppState, tx: &mpsc::Sender<Event>, key: String, name: String) {
    use crate::app::state::{BrowseColumn, BrowseDetails, BrowseItem, PlayAllRow};
    let Some(nav) = state.browse_nav() else {
        return;
    };
    let parent = nav.focused_column;
    if !matches!(nav.selected_item(),Some(BrowseItem::ArtistInfo{artist_key,..}) if artist_key==&key)
    {
        return;
    }
    if nav.columns.get(parent+1).is_some_and(|column|matches!(&column.details,Some(BrowseDetails::Artist(info)) if info.artist_key==key)) {
        if let Some(nav) = state.browse_nav_mut() { nav.focused_column = parent + 1; }
        state.miller_reveal_right = true;
        return;
    }
    state.popups.close_all();
    let mut column = BrowseColumn::new(format!("Artist Details — {name}"), vec![]);
    column.artwork_visible = true;
    if !state.downloads.is_offline()
        && state
            .sources
            .active
            .capabilities()
            .supports(crate::library::capabilities::Feature::ArtistRadio)
    {
        column.play_all_row = Some(PlayAllRow::ArtistRadio {
            artist_key: key.clone(),
            artist_name: name.clone(),
        });
    }
    column.details = Some(BrowseDetails::Artist(column_info(state, tx, key, name)));
    state.initialize_column_artwork(&mut column);
    if let Some(nav) = state.browse_nav_mut() {
        nav.drill_column(column, false);
    }
    state.miller_reveal_right = true;
    state.tui_column_header = None;
}
pub fn cycle_info_photo(state: &mut AppState, delta: i8) {
    use crate::app::state::BrowseDetails;
    let Some(index) = state.artist_info_column_index() else {
        return;
    };
    let Some(BrowseDetails::Artist(info)) = state
        .browse_nav_mut()
        .and_then(|nav| nav.columns.get_mut(index))
        .and_then(|column| column.details.as_mut())
    else {
        return;
    };
    if info.document.images.is_empty() {
        return;
    }
    info.image_index = (info.image_index as isize + isize::from(delta))
        .rem_euclid(info.document.images.len() as isize) as usize;
    let key = info.artist_key.clone();
    let data = info.document.images[info.image_index].data.clone();
    state.artwork.insert_grid_art(key, data);
}

pub(crate) async fn load_navidrome(
    session: &crate::app::sources::navidrome::Session,
    key: &str,
    name: &str,
    cached_mbid: Option<String>,
    generation: u64,
    events: &mpsc::Sender<Event>,
) -> Result<Biography, String> {
    let performer = if key.starts_with("track_artist:") {
        match session.client.performer(name).await {
            Ok(artist) => artist.map(|a| session.artist(a)),
            Err(error) => {
                tracing::debug!(%error, "Navidrome performer lookup failed");
                None
            }
        }
    } else {
        None
    };
    let cover = performer.as_ref().and_then(|a| a.thumb.clone());
    if let Some(thumb) = &cover {
        let request =
            crate::app::action::SystemAction::LoadAlbumArt(vec![(key.to_owned(), thumb.clone())]);
        let _ = events
            .send(Event::for_library(
                generation,
                Event::Effect(request.into()),
            ))
            .await;
    }
    let provider_key = performer.as_ref().map_or(key, |a| a.rating_key.as_str());
    let mbid = cached_mbid.or_else(|| performer.as_ref()?.music_brainz_id.clone());
    let result = load_navidrome_with(session, provider_key, mbid, |mbid| async move {
        biography::wikipedia_for_artist(name, mbid.as_deref()).await
    })
    .await;
    let mut document = match result {
        Ok(document) => document,
        Err(text) if cover.is_some() => Biography {
            unavailable: true,
            text,
            ..Default::default()
        },
        Err(error) => return Err(error),
    };
    document.cover_art = cover;
    Ok(document)
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
    let mut related_artists = Vec::new();
    let mut photo_url = None;
    let provider = async {
        let id = session.id(key)?;
        let params = [
            ("id", id),
            ("count", "20".into()),
            ("includeNotPresent", "false".into()),
        ];
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
        related_artists = crate::app::sources::navidrome::album_artists::parse(session, key, info);
        photo_url = artist_info_url(&info["largeImageUrl"]);
        let raw = info["biography"].as_str().unwrap_or_default();
        let text = biography::biography_text(raw);
        let (prose, source) = text
            .rsplit_once("Source:")
            .map_or((text.as_str(), None), |(body, credit)| {
                (body, Some(credit.trim()))
            });
        anyhow::ensure!(
            prose.chars().any(char::is_alphabetic),
            "No biography supplied"
        );
        let text = match source {
            Some(credit) if !credit.is_empty() => {
                format!("{}\n\nSource: {credit}", prose.trim_end())
            }
            _ => text,
        };
        Ok::<_, anyhow::Error>(Biography {
            text,
            source_url: artist_info_url(&info["lastFmUrl"]).map(|url| url.to_string()),
            ..Default::default()
        })
    }
    .await;
    let prose = async {
        match provider {
            Ok(document) => Ok(document),
            Err(issue) => wikipedia(fallback_mbid).await.map_err(|error| {
                unavailable_message(Some(&format!("{issue:#}")), &format!("{error:#}"))
            }),
        }
    };
    let (document, photo) = tokio::join!(prose, artist_photo(session, photo_url));
    let mut document = match document {
        Ok(document) => document,
        Err(text) if photo.is_some() || !related_artists.is_empty() => Biography {
            unavailable: true,
            text,
            ..Default::default()
        },
        Err(error) => return Err(error),
    };
    document.related_artists = related_artists;
    if let Some(photo) = photo {
        document.images.push(photo);
    }
    Ok(document)
}

fn artist_info_url(value: &serde_json::Value) -> Option<reqwest::Url> {
    reqwest::Url::parse(value.as_str()?).ok().filter(|url| {
        matches!(url.scheme(), "https" | "http")
            && url.username().is_empty()
            && url.password().is_none()
    })
}

async fn artist_photo(
    session: &crate::app::sources::navidrome::Session,
    url: Option<reqwest::Url>,
) -> Option<BioImage> {
    let url = url?;
    // Provider image URLs are fetched without Navidrome credentials.
    match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        session.client.bytes(url.clone(), 8 * 1024 * 1024),
    )
    .await
    {
        Ok(Ok(data)) if !data.is_empty() => Some(BioImage {
            key: url.to_string(),
            data,
            caption: "Artist image supplied by Navidrome".into(),
        }),
        Ok(Err(error)) => {
            tracing::debug!(%error, "Navidrome artist image unavailable");
            None
        }
        Err(_) => {
            tracing::debug!("Navidrome artist image timed out");
            None
        }
        Ok(Ok(_)) => None,
    }
}

fn unavailable_message(navidrome: Option<&str>, wikipedia: &str) -> String {
    tracing::debug!(navidrome, wikipedia, "Artist biography unavailable");
    "No biography available".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artist_column_reuses_a_completed_miss_and_shows_search_until_prose_arrives() {
        let mut state = AppState::new();
        let (events, _receiver) = mpsc::channel(1);
        state.artist_bios.insert(
            "artist",
            &Biography {
                unavailable: true,
                ..Default::default()
            },
        );
        let mut column = column_info(&mut state, &events, "artist".into(), "Artist".into());
        assert!(column.task.is_none());
        assert!(column.needs_web_search());
        assert!(column.excerpt().is_empty());
        column.document = Biography {
            text: "Artist biography".into(),
            ..Default::default()
        };
        assert!(!column.needs_web_search());
        assert_eq!(column.excerpt(), "Artist biography");
    }

    use serde_json::json;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
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
        let photo_available = info["largeImageUrl"] == "available-image";
        if photo_available || info["largeImageUrl"] == "fixture-image" {
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
                let status = if image && !photo_available {
                    503
                } else {
                    status
                };
                let body =
                    json!({"subsonic-response":{"status":"ok","artistInfo2":info}}).to_string();
                let body = if image {
                    "P3\n1 1\n255\n255 0 0"
                } else {
                    &body
                };
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
    async fn related_artists_survive_missing_biography_and_exclude_self_and_duplicates() {
        let (session,_,task)=server(json!({"similarArtist":[{"id":"artist","name":"Self"},{"id":"b","name":"Related B"},{"id":"b","name":"Duplicate"},{"id":"-1","name":"Not in library"}]}),200).await;
        let document = load_navidrome_with(&session, &session.key("artist"), None, |_| async {
            anyhow::bail!("No matching biography")
        })
        .await
        .unwrap();
        assert_eq!(document.text, "No biography available");
        assert_eq!(document.related_artists.len(), 1);
        assert_eq!(document.related_artists[0].rating_key, session.key("b"));
        task.abort();
    }
    #[tokio::test]
    async fn artist_biography_displays_the_credit_returned_by_the_server() {
        let (session, _, task) = server(
            json!({"biography":"Artist biography.<br><br>Source: Apple Music"}),
            200,
        )
        .await;
        let document = load_navidrome_with(&session, &session.key("artist"), None, |_| async {
            panic!("The attributed biography is usable")
        })
        .await
        .unwrap();
        assert_eq!(document.text, "Artist biography.\n\nSource: Apple Music");
        task.abort();
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
        assert_eq!(document.text, "An artist biography.");
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        task.abort();
    }

    #[tokio::test]
    async fn missing_or_failed_server_falls_back_to_wikipedia() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        for (info, status, expected_mbid) in [
            (json!({"biography":"  ","musicBrainzId":id}), 200, Some(id)),
            (json!({}), 200, None),
            (
                json!({"biography":"<p>:</p>Source: Apple Music"}),
                200,
                None,
            ),
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
        for (info, status) in [(json!({"biography":" <br> "}), 200), (json!({}), 503)] {
            let (session, _, task) = server(info, status).await;
            let error = load_navidrome_with(&session, &session.key("artist"), None, |_| {
                std::future::ready(Err(anyhow::anyhow!("Wikipedia unavailable")))
            })
            .await
            .unwrap_err();
            assert_eq!(error, "No biography available");
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
        assert_eq!(error, "No biography available");
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

    #[tokio::test]
    async fn portrait_survives_missing_provider_and_wikipedia_prose() {
        let (session, _, task) = server(json!({"largeImageUrl":"available-image"}), 200).await;
        let document = load_navidrome_with(&session, &session.key("artist"), None, |_| async {
            anyhow::bail!("No matching biography")
        })
        .await
        .unwrap();
        assert!(!document.has_prose());
        let image = image::load_from_memory(&document.images[0].data)
            .unwrap()
            .to_rgb8();
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0]);
        task.abort();
    }
}
