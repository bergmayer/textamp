//! Portable playlist metadata; exported paths never contain authentication URLs.
use crate::library::models::Track;
use anyhow::{Result, ensure};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    M3u8,
    Csv,
}
impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::M3u8 => "m3u8",
            Self::Csv => "csv",
        }
    }
}
fn line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}
fn path(track: &Track) -> Result<String> {
    let source = match &track.origin {
        crate::library::track::TrackOrigin::Folder { path, .. } => Some(path.as_str()),
        _ => track
            .media
            .iter()
            .flat_map(|media| &media.part)
            .find_map(|part| part.file.as_deref()),
    }
    .unwrap_or_default();
    let windows_path = source.as_bytes().get(1) == Some(&b':')
        && source
            .as_bytes()
            .get(2)
            .is_some_and(|byte| matches!(byte, b'/' | b'\\'));
    if let Some(url) = reqwest::Url::parse(source).ok().filter(|_| !windows_path) {
        ensure!(
            url.scheme() == "file",
            "Export requires file paths, not authenticated streaming URLs"
        );
        ensure!(
            url.username().is_empty() && url.password().is_none() && url.query().is_none(),
            "Exported file URLs cannot contain credentials or query parameters"
        );
    }
    Ok(source.to_owned())
}
fn csv(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}
pub fn encode(tracks: &[Track], format: Format) -> Result<String> {
    ensure!(!tracks.is_empty(), "No tracks to export");
    let mut output = match format {
        Format::M3u8 => "#EXTM3U\n".to_owned(),
        Format::Csv => "Title,Artist,Album,Duration (seconds),Path\r\n".to_owned(),
    };
    for track in tracks {
        let path = match path(track) {
            Ok(path) => path,
            Err(_) if format == Format::Csv => String::new(),
            Err(error) => return Err(error),
        };
        match format {
            Format::M3u8 => {
                ensure!(
                    !path.contains(['\r', '\n']),
                    "Track path contains a line break; export as CSV instead"
                );
                let path = if path.starts_with('#') {
                    format!("./{path}")
                } else {
                    path
                };
                ensure!(
                    !path.is_empty(),
                    "{} has no file path; export as CSV instead",
                    track.title
                );
                output.push_str(&format!(
                    "#EXTINF:{},{} - {}\n{}\n",
                    track
                        .duration
                        .map(|ms| (ms / 1000).to_string())
                        .unwrap_or_else(|| "-1".into()),
                    line(track.track_artist()),
                    line(&track.title),
                    path
                ));
            }
            Format::Csv => output.push_str(&format!(
                "{},{},{},{},{}\r\n",
                csv(&track.title),
                csv(track.track_artist()),
                csv(track.album_name()),
                track
                    .duration
                    .map(|ms| format!("{:.3}", ms as f64 / 1000.))
                    .unwrap_or_default(),
                csv(&path)
            )),
        }
    }
    Ok(output)
}
pub fn write(path: &Path, contents: &[u8]) -> Result<()> {
    use std::io::Write;
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(contents)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Match the document against the active catalog before creating anything on the server.
pub fn read_playlist(file: &Path, catalog: &[Track]) -> Result<(String, Vec<String>)> {
    use anyhow::{Context, bail};
    use std::{collections::HashMap, io::Read};
    const MAX_BYTES: u64 = 16 * 1024 * 1024;
    let mut contents = String::new();
    ensure!(
        std::fs::metadata(file)?.is_file(),
        "Choose a regular playlist file"
    );
    std::fs::File::open(file)?
        .take(MAX_BYTES + 1)
        .read_to_string(&mut contents)
        .context("Read playlist as UTF-8")?;
    ensure!(
        contents.len() as u64 <= MAX_BYTES,
        "Playlist exceeds 16 MiB"
    );
    let text = contents.trim_start_matches('\u{feff}');
    let name = file
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.trim().is_empty())
        .context("Playlist needs a filename")?;
    #[derive(Default)]
    struct Entry {
        path: String,
        title: String,
        artist: String,
        album: String,
    }
    let entries: Vec<Entry> = match file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "m3u" | "m3u8" => text
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.starts_with('#'))
            .map(|s| Entry {
                path: s.into(),
                ..Default::default()
            })
            .collect(),
        "csv" => {
            let mut reader = csv::Reader::from_reader(text.as_bytes());
            let header = reader.headers()?.clone();
            let column = |name: &str| {
                header
                    .iter()
                    .position(|h| h.trim().eq_ignore_ascii_case(name))
            };
            let (path, title, artist, album) = (
                column("Path"),
                column("Title"),
                column("Artist"),
                column("Album"),
            );
            ensure!(
                path.is_some() || title.is_some(),
                "CSV needs a Path or Title column"
            );
            reader
                .records()
                .map(|record| {
                    let record = record?;
                    let value = |index: Option<usize>| {
                        index.and_then(|i| record.get(i)).unwrap_or("").to_owned()
                    };
                    Ok(Entry {
                        path: value(path),
                        title: value(title),
                        artist: value(artist),
                        album: value(album),
                    })
                })
                .collect::<Result<_>>()?
        }
        _ => bail!("Choose an M3U, M3U8 or CSV playlist"),
    };
    ensure!(!entries.is_empty(), "The playlist contains no tracks");
    ensure!(
        !catalog.is_empty(),
        "Load the library track catalog before importing"
    );
    // Store only keys that occur in this document. A second catalog identity marks a match as ambiguous.
    #[derive(Clone, Copy, Default)]
    enum Match<'a> {
        #[default]
        Missing,
        Track(&'a str),
        Ambiguous,
    }
    impl<'a> Match<'a> {
        fn include(&mut self, key: &'a str) {
            *self = match *self {
                Self::Missing => Self::Track(key),
                Self::Track(previous) if previous == key => *self,
                _ => Self::Ambiguous,
            };
        }
    }
    let mut paths = HashMap::new();
    let mut titles = HashMap::new();
    for e in &entries {
        if !e.path.is_empty() {
            let key = normalized_path(&e.path);
            paths.insert(key.clone(), Match::Missing);
            if Path::new(&key).is_relative()
                && let Some(parent) = file.parent()
            {
                paths.insert(
                    normalized_path(&parent.join(key).to_string_lossy()),
                    Match::Missing,
                );
            }
        }
        if !e.title.is_empty() {
            titles.entry(e.title.as_str()).or_insert_with(Vec::new);
        }
    }
    for t in catalog {
        if let Some(matches) = titles.get_mut(t.title.as_str()) {
            matches.push(t);
        }
        if !paths.is_empty()
            && let Ok(p) = path(t)
            && !p.is_empty()
            && let Some(slot) = paths.get_mut(&normalized_path(&p))
        {
            slot.include(&t.rating_key);
        }
    }
    let mut keys = Vec::with_capacity(entries.len());
    for (i, e) in entries.iter().enumerate() {
        let key = normalized_path(&e.path);
        let mut found = paths.get(&key).copied().unwrap_or_default();
        if matches!(found, Match::Missing)
            && !key.is_empty()
            && Path::new(&key).is_relative()
            && let Some(parent) = file.parent()
        {
            found = paths
                .get(&normalized_path(&parent.join(key).to_string_lossy()))
                .copied()
                .unwrap_or_default();
        }
        if matches!(found, Match::Missing)
            && let Some(tracks) = titles.get(e.title.as_str())
        {
            let matches = tracks.iter().filter(|t| {
                (e.artist.is_empty() || t.track_artist() == e.artist)
                    && (e.album.is_empty() || t.album_name() == e.album)
            });
            for track in matches {
                found.include(&track.rating_key);
            }
        }
        match found {
            Match::Track(k) => keys.push(k.to_owned()),
            Match::Ambiguous => bail!("Playlist entry {} matches multiple library tracks", i + 1),
            Match::Missing => bail!("Playlist entry {} has no match in this library", i + 1),
        }
    }
    Ok((name.to_owned(), keys))
}

fn normalized_path(value: &str) -> String {
    let value = reqwest::Url::parse(value)
        .ok()
        .filter(|url| url.scheme() == "file")
        .and_then(|url| url.to_file_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| value.to_owned())
        .replace('\\', "/");
    let mut parts = Vec::new();
    for part in value.split('/') {
        match part {
            "." | "" => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    format!(
        "{}{}",
        if value.starts_with('/') { "/" } else { "" },
        parts.join("/")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exports_order_unicode_duplicates_and_quoted_metadata() {
        let mut track = Track::from_folder("fixture", "/Music/日本/one.flac");
        track.title = "One, \"two\"".into();
        track.grandparent_title = Some("An artist".into());
        track.duration = Some(123400);
        let tracks = vec![track.clone(), track];
        let m3u = encode(&tracks, Format::M3u8).unwrap();
        assert_eq!(m3u.matches("/Music/日本/one.flac").count(), 2);
        assert!(m3u.starts_with("#EXTM3U\n#EXTINF:123,"));
        let csv = encode(&tracks, Format::Csv).unwrap();
        assert!(csv.contains("\"One, \"\"two\"\"\""));
        assert!(csv.contains("123.400"));
    }
    #[test]
    fn refuses_streaming_credentials_and_m3u_without_paths() {
        let missing = Track::default();
        assert!(encode(std::slice::from_ref(&missing), Format::M3u8).is_err());
        assert!(encode(&[missing], Format::Csv).is_ok());
        let remote = Track::from_folder(
            "fixture",
            "https://user:password@example.test/stream?token=secret",
        );
        assert!(encode(&[remote], Format::M3u8).is_err());
    }
    #[test]
    fn leading_hash_paths_are_playable_in_m3u_and_preserved_in_csv() {
        let track = Track::from_folder("fixture", "#13 Artist/Album/Track.flac");
        assert!(
            encode(std::slice::from_ref(&track), Format::M3u8)
                .unwrap()
                .contains("\n./#13 Artist/Album/Track.flac\n")
        );
        assert!(
            encode(&[track], Format::Csv)
                .unwrap()
                .contains("\"#13 Artist/Album/Track.flac\"")
        );
    }
}
