//! Shared playback metadata and server serialization fields.
use crate::util::serde_helpers::from_str_or_num_opt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum TrackOrigin {
    Navidrome {
        source_id: String,
        song_id: String,
    },
    Folder {
        source_id: String,
        path: String,
    },
    #[default]
    #[serde(other)]
    Unavailable,
}

/// Track metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub view_count: Option<u64>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub parent_index: Option<u32>,
    #[serde(default)]
    pub origin: TrackOrigin,
    #[serde(default)]
    pub rating_key: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub lyrics: Option<String>,
    /// All track-level genre tags, in source order.
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub parent_title: Option<String>,
    #[serde(default)]
    pub grandparent_title: Option<String>,
    #[serde(default)]
    pub parent_rating_key: Option<String>,
    #[serde(default)]
    pub grandparent_rating_key: Option<String>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub index: Option<u32>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub year: Option<u16>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub parent_year: Option<u16>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub duration: Option<u64>,
    #[serde(default, rename = "Media")]
    pub media: Vec<Media>,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub parent_thumb: Option<String>,
    #[serde(default)]
    pub grandparent_thumb: Option<String>,
    /// Track-level artist name (used by server for compilation tracks).
    #[serde(default)]
    pub original_title: Option<String>,
}

impl Track {
    pub fn folder_cover_key(source_id: &str, directory: &str) -> String {
        format!(
            "folder-cover:{source_id}:{}",
            urlencoding::encode(directory)
        )
    }
    pub fn from_folder(source_id: &str, path: &str) -> Self {
        let identity = format!("folder:{source_id}:{}", urlencoding::encode(path));
        let file = std::path::Path::new(path);
        Self {
            origin: TrackOrigin::Folder {
                source_id: source_id.into(),
                path: path.into(),
            },
            parent_rating_key: Some(Self::folder_cover_key(
                source_id,
                path.rsplit_once('/').map_or("", |(dir, _)| dir),
            )),
            key: identity.clone(),
            rating_key: identity,
            title: file
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            parent_title: file
                .parent()
                .and_then(|p| p.file_name())
                .map(|p| p.to_string_lossy().into_owned()),
            media: vec![Media {
                part: vec![MediaPart {
                    file: Some(path.into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// Read embedded tags and declared duration, not sonic/AI analysis. Kept
    /// off the UI thread by the source preparation worker.
    pub fn read_file_metadata(
        &mut self,
        path: &std::path::Path,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(self.read_tags(path, true)?.0)
    }

    pub(crate) fn read_tags(
        &mut self,
        path: &std::path::Path,
        playback: bool,
    ) -> anyhow::Result<(Option<Vec<u8>>, bool)> {
        use symphonia::core::{io::MediaSourceStream, meta::StandardTagKey, probe::Hint};
        let file = std::fs::File::open(path)?;
        let mut hint = Hint::new();
        if let Some(ext) = self
            .file_name()
            .and_then(|s| std::path::Path::new(s).extension())
            .and_then(|s| s.to_str())
        {
            hint.with_extension(ext);
        }
        let mut probed = symphonia::default::get_probe().format(
            &hint,
            MediaSourceStream::new(Box::new(file), Default::default()),
            &Default::default(),
            &Default::default(),
        )?;
        if let Some(track) = probed.format.default_track()
            && let (Some(frames), Some(time_base)) =
                (track.codec_params.n_frames, track.codec_params.time_base)
        {
            let time = time_base.calc_time(frames);
            self.duration = Some(time.seconds.saturating_mul(1000) + (time.frac * 1000.0) as u64);
        }
        let mut revisions = Vec::new();
        if let Some(metadata) = probed.metadata.get().and_then(|m| m.current().cloned()) {
            revisions.push(metadata);
        }
        if let Some(metadata) = probed.format.metadata().current().cloned() {
            revisions.push(metadata);
        }
        let mut artwork = None;
        let mut tagged_album = false;
        let mut tagged_artist = false;
        for revision in revisions {
            if playback && artwork.is_none() {
                artwork = revision
                    .visuals()
                    .iter()
                    .find(|image| image.data.len() <= 8 * 1024 * 1024)
                    .map(|image| image.data.to_vec());
            }
            for tag in revision.tags() {
                let value = crate::util::sanitize_display_text(&tag.value.to_string()).into_owned();
                if value.trim().is_empty() {
                    continue;
                }
                match tag.std_key {
                    Some(StandardTagKey::Lyrics) => self.lyrics = Some(value),
                    Some(StandardTagKey::TrackTitle) => self.title = value,
                    Some(StandardTagKey::Album) => {
                        tagged_album = true;
                        self.parent_title = Some(value);
                    }
                    Some(StandardTagKey::Artist) => self.original_title = Some(value),
                    Some(StandardTagKey::AlbumArtist) => {
                        tagged_artist = true;
                        self.grandparent_title = Some(value);
                    }
                    Some(StandardTagKey::Genre) if !self.genres.contains(&value) => {
                        self.genres.push(value);
                    }
                    _ => {}
                }
            }
        }
        if playback && self.grandparent_title.is_none() {
            self.grandparent_title = self.original_title.clone();
        }
        Ok((artwork, tagged_artist && tagged_album))
    }

    /// Get track-level artist (original_title), falling back to album artist.
    pub fn track_artist(&self) -> &str {
        match self.original_title.as_deref() {
            Some(s) if !s.is_empty() => s,
            _ => self.artist_name(),
        }
    }

    pub fn distinct_album_artist(&self) -> Option<&str> {
        self.grandparent_title
            .as_deref()
            .map(str::trim)
            .filter(|artist| {
                !artist.is_empty() && !artist.eq_ignore_ascii_case(self.track_artist().trim())
            })
    }

    /// Get album name (handles None and empty string).
    pub fn album_name(&self) -> &str {
        match self.parent_title.as_deref() {
            Some(s) if !s.is_empty() => s,
            _ => "Unknown Album",
        }
    }

    /// Get artist name (handles None and empty string).
    pub fn artist_name(&self) -> &str {
        match self.grandparent_title.as_deref() {
            Some(s) if !s.is_empty() => s,
            _ => "Unknown Artist",
        }
    }

    /// Get filename from the first media part's file path (with extension).
    pub fn file_name(&self) -> Option<&str> {
        self.media
            .first()
            .and_then(|m| m.part.first())
            .and_then(|p| p.file.as_deref())
            .and_then(|f| f.rsplit('/').next())
            // Also handle Windows-style backslash paths
            .map(|f| f.rsplit('\\').next().unwrap_or(f))
    }

    /// Get track number.
    pub fn track_number(&self) -> u32 {
        self.index.unwrap_or(0)
    }

    /// Get duration in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.duration.unwrap_or(0)
    }

    /// Get best thumbnail (track > album > artist).
    pub fn best_thumb(&self) -> Option<&str> {
        [
            self.thumb.as_deref(),
            self.parent_thumb.as_deref(),
            self.grandparent_thumb.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find(|thumb| !thumb.is_empty())
    }
}

/// Media container for a track.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub id: Option<u64>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub duration: Option<u64>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub bitrate: Option<u32>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub audio_channels: Option<u8>,
    #[serde(default)]
    pub audio_codec: Option<String>,
    #[serde(default)]
    pub container: Option<String>,
    #[serde(default, rename = "Part")]
    pub part: Vec<MediaPart>,
}

/// Individual media file/part.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaPart {
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub id: Option<u64>,
    #[serde(default)]
    pub key: String,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub duration: Option<u64>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default, deserialize_with = "from_str_or_num_opt")]
    pub size: Option<u64>,
    #[serde(default)]
    pub container: Option<String>,
}
