use super::*;
use futures::StreamExt;
use reqwest::{Client, Method, RequestBuilder, Url};
use tokio::io::AsyncWriteExt;

pub(super) fn base_url(value: &str) -> Result<Url> {
    let mut url = Url::parse(value).context("Invalid WebDAV URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("WebDAV requires an HTTP(S) folder URL without credentials, query, or fragment");
    }
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}

fn endpoint(source: &FolderSource, path: &str, directory: bool) -> Result<Url> {
    let FolderLocation::Webdav { url, .. } = &source.location else {
        unreachable!()
    };
    let mut url = base_url(url)?;
    relative_path(path)?;
    if !path.is_empty() {
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Invalid WebDAV base"))?
            .pop_if_empty()
            .extend(path.split('/'));
    }
    if directory && !url.path().ends_with('/') {
        url.path_segments_mut().unwrap().push("");
    }
    Ok(url)
}

async fn request(source: &FolderSource, method: Method, url: Url) -> Result<RequestBuilder> {
    let FolderLocation::Webdav { password_env, .. } = &source.location else {
        unreachable!()
    };
    let id = source.id.clone();
    let env = password_env.clone();
    let password = tokio::task::spawn_blocking(move || source_secret(&id, &env)).await??;
    authenticated_request(source, method, url, password.as_deref().map(String::as_str))
}

fn authenticated_request(
    source: &FolderSource,
    method: Method,
    url: Url,
    password: Option<&str>,
) -> Result<RequestBuilder> {
    let FolderLocation::Webdav { username, .. } = &source.location else {
        unreachable!()
    };
    // Redirects may cross credential/root boundaries. Require the configured
    // canonical folder URL instead of following them with authentication.
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()?;
    let mut request = client.request(method, url);
    if let Some(username) = username {
        request = request.basic_auth(username, password);
    }
    Ok(request)
}

pub(super) async fn list(source: &FolderSource, path: &str) -> Result<Vec<FolderEntry>> {
    let url = endpoint(source, path, true)?;
    let request = request(source, Method::from_bytes(b"PROPFIND")?, url.clone()).await?;
    list_request(request, url, path).await
}

/// Validate a draft connection without writing credentials or configuration.
pub async fn check(source: &FolderSource, password: &str) -> Result<()> {
    anyhow::ensure!(
        matches!(source.location, FolderLocation::Webdav { .. }),
        "Expected a WebDAV source"
    );
    source.validate()?;
    let url = endpoint(source, "", true)?;
    let request = authenticated_request(
        source,
        Method::from_bytes(b"PROPFIND")?,
        url.clone(),
        Some(password),
    )?;
    list_request(request, url, "").await?;
    Ok(())
}

async fn send(request: RequestBuilder, url: &Url) -> Result<reqwest::Response> {
    request.send().await.map_err(|error| {
        let connect = error.is_connect();
        let error = anyhow::Error::new(error.without_url());
        if connect && url.scheme() == "https" {
            error.context("Cannot connect over HTTPS. Check the scheme and port: a plain-HTTP server needs http://. No automatic downgrade was attempted")
        } else {
            error.context("Cannot reach the WebDAV server; check the address and network")
        }
    })
}

async fn list_request(request: RequestBuilder, url: Url, path: &str) -> Result<Vec<FolderEntry>> {
    let request = request
        .header("Depth", "1").header("Content-Type", "application/xml; charset=utf-8")
        .body(r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:displayname/></d:prop></d:propfind>"#);
    let response = send(request, &url).await?;
    match response.status().as_u16() {
        401 => bail!("WebDAV sign-in failed (HTTP 401); check username and password"),
        403 => bail!("This account does not have permission to list that WebDAV folder (HTTP 403)"),
        404 => bail!("WebDAV folder not found (HTTP 404); check the URL path"),
        _ => {}
    }
    if response.status().as_u16() != 207 {
        bail!("WebDAV listing returned HTTP {}", response.status());
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.without_url())?;
        if body.len() + chunk.len() > 8 * 1024 * 1024 {
            bail!("WebDAV listing exceeds 8 MiB");
        }
        body.extend_from_slice(&chunk);
    }
    parse_listing(
        &url,
        path,
        std::str::from_utf8(&body).context("WebDAV listing is not UTF-8")?,
    )
}

fn dav(node: roxmltree::Node<'_, '_>, name: &str) -> bool {
    node.has_tag_name(("DAV:", name))
}

pub(crate) fn parse_listing(
    request_url: &Url,
    parent: &str,
    xml: &str,
) -> Result<Vec<FolderEntry>> {
    let document = roxmltree::Document::parse(xml).context("Invalid WebDAV XML")?;
    if !dav(document.root_element(), "multistatus") {
        bail!("Expected WebDAV multistatus");
    }
    let request_path = urlencoding::decode(request_url.path())?;
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for response in document
        .root_element()
        .children()
        .filter(|n| dav(*n, "response"))
    {
        let href = response
            .children()
            .find(|n| dav(*n, "href"))
            .and_then(|n| n.text())
            .context("WebDAV entry is missing href")?;
        let url = request_url.join(href).context("Invalid WebDAV entry URL")?;
        if url.origin() != request_url.origin() || url.query().is_some() || url.fragment().is_some()
        {
            bail!("WebDAV entry leaves the configured origin");
        }
        let decoded = urlencoding::decode(url.path())?;
        if decoded.trim_end_matches('/') == request_path.trim_end_matches('/') {
            continue;
        }
        let name = decoded
            .strip_prefix(request_path.as_ref())
            .context("WebDAV entry leaves the requested folder")?
            .trim_end_matches('/');
        let path = child_path(parent, name)?;
        let prop = response
            .children()
            .filter(|n| dav(*n, "propstat"))
            .find_map(|stat| {
                let status = stat.children().find(|n| dav(*n, "status"))?.text()?;
                if status.split_whitespace().nth(1) != Some("200") {
                    return None;
                }
                stat.children().find(|n| dav(*n, "prop"))
            })
            .context("WebDAV could not read an entry's properties")?;
        let resource = prop
            .children()
            .find(|n| dav(*n, "resourcetype"))
            .context("WebDAV entry lacks resource type")?;
        if seen.insert(path.clone()) {
            entries.push(FolderEntry {
                path,
                name: name.to_string(),
                directory: resource.children().any(|n| dav(n, "collection")),
            });
        }
        if entries.len() > MAX_ENTRIES {
            bail!("WebDAV folder exceeds {MAX_ENTRIES} entries");
        }
    }
    Ok(entries)
}

pub(super) async fn file(source: &FolderSource, path: &str, limit: u64) -> Result<MediaFile> {
    let url = endpoint(source, path, false)?;
    let response = send(request(source, Method::GET, url.clone()).await?, &url).await?;
    if !response.status().is_success() {
        bail!("WebDAV download returned HTTP {}", response.status());
    }
    if response.content_length().is_some_and(|size| size > limit) {
        bail!("Remote file exceeds the size limit");
    }
    let temporary = tempfile::NamedTempFile::new().context("Cannot create media spool")?;
    let mut output = tokio::fs::File::from_std(temporary.reopen()?);
    let mut stream = response.bytes_stream();
    let mut total = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.without_url())?;
        total += chunk.len() as u64;
        if total > limit {
            bail!("Remote file exceeds the size limit");
        }
        output
            .write_all(&chunk)
            .await
            .context("Cannot spool media")?;
    }
    output.flush().await?;
    Ok(MediaFile::temporary(temporary))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn response(href: &str, status: &str) -> String {
        format!(
            r#"<multistatus xmlns="DAV:"><response><href>{href}</href><propstat><prop><resourcetype/></prop><status>HTTP/1.1 {status}</status></propstat></response></multistatus>"#
        )
    }
    #[test]
    fn listing_rejects_out_of_root_cross_origin_nested_and_failed_entries() {
        let url = Url::parse("https://example.test/music/").unwrap();
        for href in [
            "/secret/a.flac",
            "https://evil.test/music/a.flac",
            "/music/album/a.flac",
            "/music/%2e%2e/a.flac",
            "/music/%2Fetc%2Fpasswd",
            "/music/a.flac?token=bad",
        ] {
            assert!(
                parse_listing(&url, "", &response(href, "200 OK")).is_err(),
                "{href}"
            );
        }
        assert!(parse_listing(&url, "", &response("/music/a.flac", "403 Forbidden")).is_err());
        assert!(parse_listing(&url, "", "not xml").is_err());
        assert!(parse_listing(&url, "", "<multistatus/>").is_err());
    }
    #[test]
    fn request_encodes_names_once_without_inheriting_credentials() {
        let source = FolderSource {
            id: "dav".into(),
            name: "DAV".into(),
            location: FolderLocation::Webdav {
                url: "https://example.test/my%20music/".into(),
                username: Some("private-user".into()),
                password_env: None,
            },
        };
        assert_eq!(
            endpoint(&source, "100% song #1.flac", false)
                .unwrap()
                .as_str(),
            "https://example.test/my%20music/100%25%20song%20%231.flac"
        );
    }
}
