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
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Invalid WebDAV base"))?
            .push("");
    }
    Ok(url)
}

async fn request(source: &FolderSource, method: Method, url: Url) -> Result<RequestBuilder> {
    let FolderLocation::Webdav {
        username,
        password_env,
        ..
    } = &source.location
    else {
        unreachable!()
    };
    if username.is_none() {
        return authenticated_request(source, method, url, None);
    }
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
    static CLIENT: std::sync::OnceLock<Client> = std::sync::OnceLock::new();
    let client = if let Some(client) = CLIENT.get() {
        client.clone()
    } else {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        let _ = CLIENT.set(client.clone());
        client
    };
    let mut request = client.request(method, url);
    if let Some(username) = username {
        request = request.basic_auth(username, password);
    }
    Ok(request)
}

pub(super) async fn list(source: &FolderSource, path: &str) -> Result<Vec<FolderEntry>> {
    Ok(list_with_versions(source, path)
        .await?
        .into_iter()
        .map(|(entry, _)| entry)
        .collect())
}

/// Folder listings can validate a whole album without opening individual tracks.
pub(crate) async fn list_with_versions(
    source: &FolderSource,
    path: &str,
) -> Result<Vec<(FolderEntry, String)>> {
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

async fn list_request(
    request: RequestBuilder,
    url: Url,
    path: &str,
) -> Result<Vec<(FolderEntry, String)>> {
    let request = request
        .header("Depth", "1").header("Content-Type", "application/xml; charset=utf-8")
        .body(r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getetag/><d:getlastmodified/><d:getcontentlength/></d:prop></d:propfind>"#);
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
) -> Result<Vec<(FolderEntry, String)>> {
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
        let properties = || {
            response
                .children()
                .filter(|node| dav(*node, "propstat"))
                .filter_map(|stat| {
                    let status = stat.children().find(|node| dav(*node, "status"))?.text()?;
                    if status.split_whitespace().nth(1) != Some("200") {
                        return None;
                    }
                    stat.children().find(|node| dav(*node, "prop"))
                })
                .flat_map(|prop| prop.children())
        };
        let resource = properties()
            .find(|n| dav(*n, "resourcetype"))
            .context("WebDAV entry lacks resource type")?;
        if seen.insert(path.clone()) {
            let property = |name: &str| {
                properties()
                    .find(|node| dav(*node, name))
                    .and_then(|node| node.text())
            };
            let version = file_version(
                property("getetag"),
                property("getlastmodified"),
                property("getcontentlength").and_then(|n| n.parse().ok()),
            );
            entries.push((
                FolderEntry {
                    path,
                    name: name.to_string(),
                    directory: resource.children().any(|n| dav(n, "collection")),
                },
                version,
            ));
        }
        if entries.len() > MAX_ENTRIES {
            bail!("WebDAV folder exceeds {MAX_ENTRIES} entries");
        }
    }
    Ok(entries)
}

/// Fetch the album sidecar through the same authenticated, root-confined endpoint as audio.
pub(crate) async fn folder_cover(
    source: &FolderSource,
    directory: &str,
) -> Result<Option<Vec<u8>>> {
    let path = child_path(directory, "cover.jpg")?;
    let url = endpoint(source, &path, false)?;
    let response = send(request(source, Method::GET, url.clone()).await?, &url).await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    anyhow::ensure!(
        response.status().is_success(),
        "WebDAV artwork returned HTTP {}",
        response.status()
    );
    const LIMIT: usize = 16 * 1024 * 1024;
    anyhow::ensure!(
        response.content_length().is_none_or(|n| n <= LIMIT as u64),
        "WebDAV artwork exceeds 16 MiB"
    );
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.without_url())?;
        anyhow::ensure!(
            chunk.len() <= LIMIT - bytes.len(),
            "WebDAV artwork exceeds 16 MiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    anyhow::ensure!(
        image::guess_format(&bytes).is_ok(),
        "Invalid WebDAV cover image"
    );
    Ok(Some(bytes))
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

fn response_version(headers: &reqwest::header::HeaderMap, expected: Option<u64>) -> String {
    let header = |name| headers.get(name).and_then(|value| value.to_str().ok());
    file_version(
        header(reqwest::header::ETAG),
        header(reqwest::header::LAST_MODIFIED),
        expected,
    )
}

fn file_version(etag: Option<&str>, modified: Option<&str>, expected: Option<u64>) -> String {
    if let Some(etag) = etag.filter(|value| !value.is_empty() && !value.starts_with("W/")) {
        format!("etag:{etag}")
    } else if let (Some(modified), Some(bytes)) = (modified, expected) {
        format!("modified:{modified}:{bytes}")
    } else {
        String::new()
    }
}

/// Read validators without transferring audio. Some WebDAV servers omit HEAD support.
pub(crate) async fn version(
    source: &FolderSource,
    path: &str,
) -> Result<Option<(String, Option<u64>)>> {
    let url = endpoint(source, path, false)?;
    let response = send(request(source, Method::HEAD, url.clone()).await?, &url).await?;
    match response.status().as_u16() {
        405 | 501 => return Ok(None),
        200 => {}
        code => bail!("WebDAV could not check the current file version (HTTP {code})"),
    }
    let headers = response.headers();
    let expected = headers
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());
    Ok(Some((response_version(headers, expected), expected)))
}

/// Streams a WebDAV file directly into the caller's temporary destination.
pub(crate) async fn transfer(
    source: &FolderSource,
    path: &str,
    previous: Option<&str>,
    destination: &Path,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<Option<(u64, String, String)>> {
    let url = endpoint(source, path, false)?;
    let response = send(request(source, Method::GET, url.clone()).await?, &url).await?;
    anyhow::ensure!(
        response.status() == reqwest::StatusCode::OK,
        "WebDAV download returned HTTP {}",
        response.status()
    );
    let headers = response.headers();
    anyhow::ensure!(
        !headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value
                .split(';')
                .next()
                .is_some_and(|v| v.eq_ignore_ascii_case("text/html"))),
        "WebDAV returned a document instead of audio"
    );
    let expected = response.content_length();
    let version = response_version(headers, expected);
    if !version.is_empty() && previous == Some(version.as_str()) {
        return Ok(None);
    }
    anyhow::ensure!(
        expected.is_none_or(|bytes| bytes <= MAX_REMOTE_BYTES),
        "Remote file exceeds the size limit"
    );
    let mut output = tokio::fs::File::create(destination).await?;
    let mut stream = response.bytes_stream();
    let mut hash = md5::Context::new();
    let mut total = 0;
    progress(total, expected);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| error.without_url())?;
        total += chunk.len() as u64;
        anyhow::ensure!(
            total <= MAX_REMOTE_BYTES,
            "Remote file exceeds the size limit"
        );
        hash.consume(&chunk);
        output.write_all(&chunk).await?;
        progress(total, expected);
    }
    anyhow::ensure!(
        total > 0 && expected.is_none_or(|bytes| bytes == total),
        "WebDAV returned incomplete audio"
    );
    output.sync_all().await?;
    Ok(Some((total, version, format!("{:x}", hash.finalize()))))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn response(href: &str, status: &str) -> String {
        format!(
            r#"<multistatus xmlns="DAV:"><response><href>{href}</href><propstat><prop><resourcetype/></prop><status>HTTP/1.1 {status}</status></propstat></response></multistatus>"#
        )
    }
    #[tokio::test]
    async fn folder_cover_handles_encoded_paths_and_missing_sidecars() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let source = FolderSource {
            id: "cover-test".into(),
            name: "Albums".into(),
            location: FolderLocation::Webdav {
                url: format!("http://{}/Music/", listener.local_addr().unwrap()),
                username: None,
                password_env: None,
            },
        };
        let mut jpg = Vec::new();
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(
                &mut std::io::Cursor::new(&mut jpg),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        let expected = jpg.clone();
        let server = tokio::spawn(async move {
            for (path, status, body) in [
                ("LP%20%231/cover.jpg", "200 OK", jpg.as_slice()),
                ("cover.jpg", "404 Not Found", &[][..]),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut headers = Vec::new();
                while !headers.ends_with(b"\r\n\r\n") {
                    headers.push(socket.read_u8().await.unwrap());
                }
                assert!(headers.starts_with(format!("GET /Music/{path} HTTP/1.1").as_bytes()));
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                socket.write_all(body).await.unwrap();
            }
        });
        assert_eq!(
            folder_cover(&source, "LP #1").await.unwrap(),
            Some(expected)
        );
        assert_eq!(folder_cover(&source, "").await.unwrap(), None);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn consecutive_listings_reuse_the_http_connection() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/music/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            for _ in 0..2 {
                let mut input = Vec::new();
                loop {
                    let mut chunk = [0; 2048];
                    let read = socket.read(&mut chunk).await.unwrap();
                    assert!(read > 0, "the next listing must reuse this connection");
                    input.extend_from_slice(&chunk[..read]);
                    if let Some(end) = input.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&input[..end]);
                        let length = head
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if input.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let body = response("/music/a.flac", "200 OK");
                socket.write_all(format!("HTTP/1.1 207 Multi-Status\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        });
        let source = FolderSource {
            id: "pool-fixture".into(),
            name: "Fixture".into(),
            location: FolderLocation::Webdav {
                url: base.clone(),
                username: None,
                password_env: None,
            },
        };
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            for _ in 0..2 {
                let entries = source.list("").await.unwrap();
                assert_eq!(entries[0].name, "a.flac");
            }
            server.await.unwrap();
        })
        .await
        .expect("directory requests must share a live pooled connection");
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
