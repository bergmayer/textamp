//! External searches, including native Music search on macOS.

use anyhow::{bail, Context, Result};
use std::time::Duration;
use tokio::process::Command;

/// Target service for external search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchTarget {
    AppleMusic,
    Spotify,
    YouTube,
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

/// Open a search, returning a brief notice only when native Music was unavailable.
/// A successful handoff does not imply that the service returned any results.
pub async fn open_search(target: SearchTarget, query: &str) -> Result<Option<&'static str>> {
    if query.trim().is_empty() {
        bail!("Nothing selected to search");
    }
    // Native UI requests cannot overlap: two scripts could submit each other's
    // queries. Reject repeats instead of building up a queue of app launches.
    static SEARCH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _search = SEARCH
        .try_lock()
        .context("Another search is still opening")?;
    #[cfg(not(target_os = "macos"))]
    let notice = None;
    #[cfg(target_os = "macos")]
    let mut notice = None;
    #[cfg(target_os = "macos")]
    if target == SearchTarget::AppleMusic {
        match run_command(native_music_command(query), Duration::from_secs(15)).await {
            Ok(()) => return Ok(None),
            Err(error) => {
                tracing::warn!("Native Music search unavailable: {error:#}");
                notice = Some("Music unavailable; using web search (see Help).");
            }
        }
    }
    // open supplies the appropriate platform launchers, but run them ourselves
    // so a stuck launcher neither blocks the event loop nor survives shutdown.
    let url = generate_search_url(target, query);
    open_browser(&url).await?;
    Ok(notice)
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

#[cfg(target_os = "macos")]
fn native_music_command(query: &str) -> Command {
    let mut command = Command::new("/usr/bin/osascript");
    // Metadata is an argument, never executable AppleScript or clipboard text.
    command.args(["-e", include_str!("music_search.applescript"), "--", query]);
    command
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn native_query_is_a_literal_argument() {
        let query = "Björk \" & do shell script \"bad\" --\n日本語";
        let command = native_music_command(query);
        let args: Vec<_> = command.as_std().get_args().collect();
        assert_eq!(args[2], "--");
        assert_eq!(args[3], query);
        assert!(!args[1].to_string_lossy().contains(query));
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
