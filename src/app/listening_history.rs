//! A bounded, chronological listening history shared by all presentations.
use crate::library::models::Track;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path, sync::Arc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Play {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub time: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<Track>,
}
pub fn track_action(id: &str, kind: super::track_context::ContextKind) -> super::Action {
    super::action::QueueAction::HistoryTrack(id.to_owned(), kind).into()
}
pub type History = Arc<Vec<Play>>;
const LIMIT: usize = 512;
fn directory() -> Result<std::path::PathBuf> {
    crate::util::paths::get_data_dir("textamp").context("Missing listening history directory")
}
fn read(path: &Path) -> Result<Vec<Play>> {
    match fs::File::open(path) {
        Ok(file) => Ok(serde_json::from_reader(std::io::BufReader::new(file))?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}
pub fn load() -> Result<History> {
    Ok(Arc::new(read(
        &directory()?.join("listening-history.json"),
    )?))
}
pub fn save(play: Option<Play>) -> Result<History> {
    update(&directory()?, play)
}
fn update(dir: &Path, play: Option<Play>) -> Result<History> {
    fs::create_dir_all(dir)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("listening-history.lock"))?;
    fs4::fs_std::FileExt::lock_exclusive(&lock)?;
    let path = dir.join("listening-history.json");
    let mut rows = if play.is_some() {
        read(&path)?
    } else {
        Vec::new()
    };
    if let Some(play) = play
        && !rows.iter().any(|row| row.id == play.id)
    {
        rows.insert(0, play);
        rows.sort_by_key(|p| std::cmp::Reverse(p.time));
        rows.truncate(LIMIT);
    }
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer(&mut temp, &rows)?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(Arc::new(rows))
}
impl Play {
    pub fn resolve(&self, state: &super::AppState) -> Option<Track> {
        use crate::library::track::TrackOrigin;
        if let Some(track) = &self.track {
            let active = match &track.origin {
                TrackOrigin::Navidrome { source_id, .. } => state
                    .sources
                    .active
                    .navidrome()
                    .is_some_and(|s| &s.source.id == source_id),
                TrackOrigin::Folder { source_id, .. } => {
                    state.sources.active.folder() == Some(source_id)
                }
                TrackOrigin::Unavailable => false,
            };
            return active.then(|| track.clone());
        }
        let mut matches = state.library.all_tracks.iter().filter(|t| {
            t.title == self.title && t.track_artist() == self.artist && t.album_name() == self.album
        });
        let track = matches.next()?;
        matches.next().is_none().then(|| track.clone())
    }
    pub fn from_track(track: &Track) -> Self {
        Self {
            track: Some(track.clone()),
            id: uuid::Uuid::new_v4().to_string(),
            time: time::OffsetDateTime::now_utc().unix_timestamp(),
            title: track.title.clone(),
            artist: track.track_artist().into(),
            album: track.album_name().into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_track_keeps_its_source_and_legacy_plays_require_a_unique_match() {
        let mut app = crate::app::AppState::new();
        let track = Track::from_folder("fixture", "album/song.mp3");
        let saved: Play =
            serde_json::from_value(serde_json::to_value(Play::from_track(&track)).unwrap())
                .unwrap();
        app.sources.active = crate::app::sources::ActiveSource::Folder("fixture".into());
        assert_eq!(saved.resolve(&app).unwrap().origin, track.origin);
        app.sources.active = crate::app::sources::ActiveSource::Folder("other".into());
        assert!(saved.resolve(&app).is_none());
        let mut old = serde_json::to_value(saved).unwrap();
        old.as_object_mut().unwrap().remove("track");
        let old: Play = serde_json::from_value(old).unwrap();
        let other = Track::from_folder("other", "album/song.mp3");
        app.library.all_tracks = Arc::new(vec![other.clone()]);
        assert_eq!(old.resolve(&app).unwrap().origin, other.origin);
        app.library.all_tracks = Arc::new(vec![other.clone(), other]);
        assert!(old.resolve(&app).is_none());
    }
    #[test]
    fn repeated_tracks_are_distinct_and_clear_survives_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let entry = |n: i64| Play {
            track: None,
            id: n.to_string(),
            time: n,
            title: "Same track".into(),
            artist: "Artist".into(),
            album: "Album".into(),
        };
        let file = temp.path().join("listening-history.json");
        fs::write(
            &file,
            serde_json::to_vec(&(0..512).rev().map(entry).collect::<Vec<_>>()).unwrap(),
        )
        .unwrap();
        update(temp.path(), Some(entry(512))).unwrap();
        update(temp.path(), Some(entry(512))).unwrap();
        let loaded = read(&file).unwrap();
        assert_eq!(loaded.len(), 512);
        assert_eq!(
            (loaded.first().unwrap().time, loaded.last().unwrap().time),
            (512, 1)
        );
        update(temp.path(), None).unwrap();
        assert!(read(&file).unwrap().is_empty());
    }
}
