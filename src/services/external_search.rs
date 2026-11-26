//! Service search URLs and operating-system handoffs.
use anyhow::{Context, Result, bail};
use std::time::Duration;
use tokio::process::Command;

/// Target service for external search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchTarget {
    AppleMusic,
    Spotify,
    YouTube,
}

impl SearchTarget {
    /// External services in menu order.
    pub const ALL: [Self; 3] = [Self::AppleMusic, Self::YouTube, Self::Spotify];

    /// Display name shared by settings, commands and native handoffs.
    pub fn name(self) -> &'static str {
        match self {
            Self::AppleMusic => "Apple Music",
            Self::Spotify => "Spotify",
            Self::YouTube => "YouTube",
        }
    }
}

/// Catalog identity captured when a user invokes an external search.
#[derive(Debug, Clone)]
pub enum MusicQuery {
    Text(String),
    Artist(String),
    Album {
        artist: String,
        title: String,
    },
    Song {
        artist: String,
        title: String,
        album: Option<String>,
    },
}

impl From<&str> for MusicQuery {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl MusicQuery {
    pub fn text(&self) -> String {
        match self {
            Self::Text(text) | Self::Artist(text) => text.clone(),
            Self::Album { artist, title } | Self::Song { artist, title, .. } => {
                if artist.trim().is_empty() {
                    title.trim().into()
                } else {
                    format!("{} - {}", artist.trim(), title.trim())
                }
            }
        }
    }

    fn catalog_term(&self) -> Option<(&'static str, String)> {
        match self {
            Self::Text(_) => None,
            Self::Artist(name) => Some(("musicArtist", name.clone())),
            Self::Song { artist, title, .. } => Some(("song", format!("{artist} {title}"))),
            Self::Album { artist, title } => Some(("album", format!("{artist} {title}"))),
        }
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct CatalogItem {
    artist_name: String,
    collection_name: String,
    track_name: String,
    artist_link_url: Option<String>,
    collection_view_url: Option<String>,
    track_view_url: Option<String>,
}

fn catalog_link(query: &MusicQuery, items: &[CatalogItem]) -> Option<String> {
    let same = |a: &str, b: &str| {
        !a.trim().is_empty() && a.trim().to_lowercase() == b.trim().to_lowercase()
    };
    let mut matches = items.iter().filter_map(|item| {
        let (link, kind) = match query {
            MusicQuery::Artist(name) if same(name, &item.artist_name) => {
                (item.artist_link_url.as_ref()?, "artist")
            }
            MusicQuery::Album { artist, title }
                if same(artist, &item.artist_name) && same(title, &item.collection_name) =>
            {
                (item.collection_view_url.as_ref()?, "album")
            }
            MusicQuery::Song {
                artist,
                title,
                album,
            } if same(artist, &item.artist_name)
                && same(title, &item.track_name)
                && album
                    .as_ref()
                    .is_none_or(|name| same(name, &item.collection_name)) =>
            {
                (item.track_view_url.as_ref()?, "album")
            }
            _ => return None,
        };
        let url = reqwest::Url::parse(link).ok()?;
        (url.scheme() == "https"
            && url.host_str() == Some("music.apple.com")
            && url.path_segments()?.nth(1) == Some(kind))
        .then_some(link)
    });
    let first = matches.next()?;
    matches.all(|other| other == first).then(|| first.clone())
}

async fn apple_catalog_link(query: &MusicQuery) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Results {
        results: Vec<CatalogItem>,
    }
    let (entity, term) = query.catalog_term()?;
    let response = reqwest::Client::new()
        .get("https://itunes.apple.com/search")
        .query(&[
            ("term", term.as_str()),
            ("country", "US"),
            ("entity", entity),
            ("limit", "25"),
        ])
        .timeout(Duration::from_secs(4))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    catalog_link(query, &response.json::<Results>().await.ok()?.results)
}

pub async fn resolve(target: SearchTarget, query: MusicQuery) -> Destination {
    let mut destination = search_destination(target, &query.text());
    if target == SearchTarget::AppleMusic {
        destination.native_url = apple_catalog_link(&query).await;
    }
    destination
}

#[derive(Debug, Clone)]
pub struct Destination {
    pub target: SearchTarget,
    pub label: &'static str,
    pub native_url: Option<String>,
    pub web_url: String,
}

pub fn search_destination(target: SearchTarget, query: &str) -> Destination {
    Destination {
        target,
        label: target.name(),
        native_url: (target == SearchTarget::Spotify)
            .then(|| format!("spotify:search:{}", urlencoding::encode(query))),
        web_url: generate_search_url(target, query),
    }
}

/// Browser search URL. Use an explicit storefront to avoid Music's countryless
/// redirect changing encoded spaces into literal plus signs.
pub fn generate_search_url(target: SearchTarget, query: &str) -> String {
    let encoded = urlencoding::encode(query);
    match target {
        SearchTarget::AppleMusic => format!("https://music.apple.com/us/search?term={}", encoded),
        SearchTarget::Spotify => {
            format!("https://open.spotify.com/search/{}", encoded)
        }
        SearchTarget::YouTube => {
            format!("https://www.youtube.com/results?search_query={}", encoded)
        }
    }
}

/// Apple Music requires its native app. Other services can use web results.
/// A successful handoff does not imply that the service returned any results.
pub async fn open_search(target: SearchTarget, query: &str) -> Result<Option<&'static str>> {
    if query.trim().is_empty() {
        bail!("Nothing selected to search");
    }
    if target == SearchTarget::AppleMusic {
        #[cfg(target_os = "macos")]
        {
            let mut command = Command::new("/usr/bin/open");
            command.args(["-b", "com.apple.Music", &generate_search_url(target, query)]);
            run_command(command, Duration::from_secs(10))
                .await
                .context("Could not open the Music app")?;
            return Ok(None);
        }
        #[cfg(target_os = "windows")]
        {
            open_windows_music(&generate_search_url(target, query)).await?;
            return Ok(None);
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        bail!("Apple Music app search is unavailable on this platform");
    }
    let destination = search_destination(target, query);
    if target == SearchTarget::YouTube {
        open_browser(&destination.web_url).await?;
        return Ok(None);
    }
    #[cfg(target_os = "macos")]
    if let Some(url) = &destination.native_url {
        let mut command = Command::new("/usr/bin/open");
        command.args(["-a", "Spotify", url]);
        if run_command(command, Duration::from_secs(10)).await.is_ok() {
            return Ok(None);
        }
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(url) = &destination.native_url {
        #[cfg(target_os = "windows")]
        let available = {
            let scheme = url.split(':').next().unwrap_or_default();
            let mut command = Command::new("reg");
            command.args(["query", &format!("HKCR\\{scheme}"), "/v", "URL Protocol"]);
            run_command(command, Duration::from_secs(3)).await.is_ok()
        };
        #[cfg(not(target_os = "windows"))]
        let available = true;
        if available && open_browser(url).await.is_ok() {
            return Ok(None);
        }
    }
    open_browser(&destination.web_url).await?;
    Ok(None)
}

#[cfg(target_os = "windows")]
async fn open_windows_music(url: &str) -> Result<()> {
    use windows::{
        Foundation::Uri,
        System::{Launcher, LauncherOptions},
        core::HSTRING,
    };
    let options = LauncherOptions::new()?;
    options.SetTargetApplicationPackageFamilyName(&HSTRING::from(
        "AppleInc.AppleMusicWin_nzyj5cx40ttqa",
    ))?;
    let uri = Uri::CreateUri(&HSTRING::from(url))?;
    let launch = Launcher::LaunchUriWithOptionsAsync(&uri, &options)?;
    if !tokio::time::timeout(Duration::from_secs(10), async { launch.await })
        .await
        .context("Apple Music launch timed out")??
    {
        bail!("Could not open the Apple Music app");
    }
    Ok(())
}

/// Hand off a URL to the default browser without blocking the UI indefinitely.
pub async fn open_browser(url: &str) -> Result<()> {
    let mut failure = anyhow::anyhow!("No browser launcher available");
    for command in open::commands(url) {
        match run_command(command.into(), Duration::from_secs(10)).await {
            Ok(()) => return Ok(()),
            Err(error) => failure = error,
        }
    }
    Err(failure.context("Could not open browser"))
}

async fn run_command(mut command: Command, timeout: Duration) -> Result<()> {
    command.kill_on_drop(true);
    let output = tokio::time::timeout(timeout, command.output())
        .await
        .context("Search handoff timed out")??;
    if !output.status.success() {
        bail!(
            "Search handoff failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn catalog_search_url_preserves_unicode_ampersands_and_spaces() {
        let query = "Windy & Carl – Depths";
        let url =
            reqwest::Url::parse(&generate_search_url(SearchTarget::AppleMusic, query)).unwrap();
        assert_eq!(url.host_str(), Some("music.apple.com"));
        assert_eq!(url.path(), "/us/search");
        assert_eq!(
            url.query_pairs().find(|(key, _)| key == "term").unwrap().1,
            query
        );
    }

    #[test]
    fn song_link_requires_the_requested_release_and_a_unique_apple_destination() {
        let query = MusicQuery::Song {
            artist: "Guided By Voices".into(),
            title: "Doughnut for a Snowman".into(),
            album: Some("Let's Go Eat the Factory".into()),
        };
        let link = "https://music.apple.com/us/album/doughnut-for-a-snowman/480002450?i=480002454";
        let item = |album: &str, url: &str| CatalogItem {
            artist_name: "GUIDED BY VOICES".into(),
            track_name: "Doughnut for a Snowman".into(),
            collection_name: album.into(),
            track_view_url: Some(url.into()),
            ..Default::default()
        };
        let release = "Let's Go Eat the Factory";
        assert_eq!(catalog_link(&query, &[item("Live", link)]), None);
        assert_eq!(
            catalog_link(
                &query,
                &[item(release, "https://example.com/us/album/title/123")]
            ),
            None
        );
        assert_eq!(
            catalog_link(&query, &[item("Live", link), item(release, link)]).as_deref(),
            Some(link)
        );
        assert_eq!(
            catalog_link(
                &query,
                &[
                    item(release, link),
                    item(release, "https://music.apple.com/us/album/another/2?i=3")
                ]
            ),
            None
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn handoff_reports_failure_and_times_out() {
        run_command(Command::new("/usr/bin/true"), Duration::from_secs(1))
            .await
            .unwrap();
        assert!(
            run_command(Command::new("/usr/bin/false"), Duration::from_secs(1))
                .await
                .is_err()
        );
        let mut command = Command::new("/bin/sleep");
        command.arg("10");
        let error = run_command(command, Duration::from_millis(20))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("timed out"));
    }
}
