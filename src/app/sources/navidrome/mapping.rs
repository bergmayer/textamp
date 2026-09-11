use super::Session;
use crate::library::catalog::{Album, Artist, GenreTag, Playlist};
use crate::library::track::TrackOrigin;
use crate::library::track::{Media, MediaPart, Track};
use crate::navidrome::models as n;

fn clean(s: String) -> String {
    crate::util::sanitize_display_text(&s).into_owned()
}
impl Session {
    pub fn artist(&self, a: n::Artist) -> Artist {
        Artist {
            rating_key: self.key(&a.id),
            key: self.key(&a.id),
            title: clean(a.name),
            thumb: a.cover_art.map(|s| self.key(&s)),
            ..Default::default()
        }
    }
    pub fn album(&self, a: n::Album) -> Album {
        let mut seen = std::collections::HashSet::new();
        let names: Vec<_> = if a.genres.is_empty() {
            a.genre.into_iter().collect()
        } else {
            a.genres.into_iter().map(|g| g.name).collect()
        };
        let genres: Vec<_> = names
            .into_iter()
            .map(|s| clean(s).trim().to_owned())
            .filter(|s| !s.is_empty() && seen.insert(s.clone()))
            .map(|tag| GenreTag {
                tag,
                ..Default::default()
            })
            .collect();
        Album {
            subtype: (a.is_compilation
                || a.release_types
                    .iter()
                    .any(|kind| kind.eq_ignore_ascii_case("compilation")))
            .then(|| "compilation".to_owned()),
            mood: a
                .moods
                .into_iter()
                .map(|tag| GenreTag {
                    tag: clean(tag),
                    ..Default::default()
                })
                .collect(),
            originally_available_at: a
                .original_release_date
                .or(a.release_date)
                .and_then(|d| Some(format!("{:04}-{:02}-{:02}", d.year?, d.month?, d.day?))),
            rating_key: self.key(&a.id),
            key: self.key(&a.id),
            title: clean(a.name),
            parent_title: a.artist.map(clean),
            parent_rating_key: a.artist_id.map(|s| self.key(&s)),
            thumb: a.cover_art.map(|s| self.key(&s)),
            year: a.year,
            leaf_count: a.song_count,
            duration: a.duration.map(|s| s.saturating_mul(1000)),
            genre: genres,
            ..Default::default()
        }
    }
    pub fn track(&self, t: n::Song) -> Track {
        Track {
            view_count: t.play_count,
            parent_index: t.disc_number,
            origin: TrackOrigin::Navidrome {
                source_id: self.source.id.clone(),
                song_id: t.id.clone(),
            },
            rating_key: self.key(&t.id),
            key: self.key(&t.id),
            title: clean(t.title),
            parent_title: t.album.map(clean),
            parent_rating_key: t.album_id.map(|s| self.key(&s)),
            grandparent_title: t.album_artist.or_else(|| t.artist.clone()).map(clean),
            grandparent_rating_key: t.artist_id.map(|s| self.key(&s)),
            original_title: t.artist.map(clean),
            thumb: t.cover_art.map(|s| self.key(&s)),
            index: t.track,
            year: t.year,
            duration: t.duration.map(|s| s.saturating_mul(1000)),
            media: vec![Media {
                audio_codec: t.suffix,
                bitrate: t.bit_rate,
                part: vec![MediaPart {
                    file: t.path,
                    size: t.size,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
    }
    pub fn playlist(&self, p: n::Playlist) -> Playlist {
        Playlist {
            rating_key: self.key(&p.id),
            key: self.key(&p.id),
            title: clean(p.name),
            playlist_type: "audio".into(),
            composite: p.cover_art.map(|s| self.key(&s)),
            duration: p.duration.map(|s| s.saturating_mul(1000)),
            leaf_count: p.song_count,
            added_at: None,
            updated_at: None,
            smart: false,
        }
    }
}
