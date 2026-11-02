//! Resolve a server alias before sending any account credentials.
use anyhow::{ensure, Context, Result};
use std::time::Duration;

pub fn normalize(input: &str) -> Result<reqwest::Url> {
    let input = input.trim();
    ensure!(!input.is_empty(), "Enter a server address");
    let value = if input.contains("://") {
        input.to_owned()
    } else {
        format!("http://{input}")
    };
    let url = reqwest::Url::parse(&value).context("Invalid server address")?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Use an http(s) server address without credentials, query or fragment"
    );
    Ok(url)
}

/// Probe the API without credentials or cookies, then save its canonical base.
/// Runtime clients keep redirects disabled; credentials go only to that base.
pub async fn resolve(input: &str, endpoint: &str) -> Result<String> {
    let base = normalize(input)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.error("Too many server redirects");
            }
            let url = attempt.url();
            let downgrade = attempt
                .previous()
                .last()
                .is_some_and(|previous| previous.scheme() == "https" && url.scheme() != "https");
            if downgrade
                || !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return attempt.error("Unsafe server redirect");
            }
            attempt.follow()
        }))
        .build()?;
    let probe = reqwest::Url::parse(&format!(
        "{}/{}",
        base.as_str().trim_end_matches('/'),
        endpoint
    ))?;
    let response = client
        .get(probe)
        .send()
        .await
        .map_err(|error| error.without_url())
        .context("Could not resolve the server address")?;
    let mut url = response.url().clone();
    let path = url.path().strip_suffix(&format!("/{endpoint}"))
        .context("The address redirects to a website login instead of the music API; use the direct server address")?
        .trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url.as_str().trim_end_matches('/').to_owned())
}
