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
pub fn write(path: &Path, tracks: &[Track], format: Format) -> Result<()> {
    let contents = encode(tracks, format)?;
    use std::io::Write;
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(contents.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
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
