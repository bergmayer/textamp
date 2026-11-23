//! Persistent command recency, measured in palette openings rather than time.
use super::state::PaletteCommandKind;
use serde::{Deserialize, Serialize};

const OPENINGS: u8 = 10;
const LIMIT: usize = 64;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    recent: Vec<Recent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Recent {
    key: String,
    openings_left: u8,
}

impl History {
    /// A zero-lived entry remains visible in its tenth opening and expires at the next.
    pub fn opened(&mut self) -> bool {
        let changed = !self.recent.is_empty();
        self.recent.retain(|item| item.openings_left > 0);
        for item in &mut self.recent {
            item.openings_left -= 1;
        }
        changed
    }

    pub fn remember(&mut self, key: String) -> bool {
        if self
            .recent
            .first()
            .is_some_and(|item| item.key == key && item.openings_left == OPENINGS)
        {
            return false;
        }
        self.recent.retain(|item| item.key != key);
        self.recent.insert(
            0,
            Recent {
                key,
                openings_left: OPENINGS,
            },
        );
        self.recent.truncate(LIMIT);
        true
    }

    pub fn rank(&self, key: &str) -> Option<usize> {
        self.recent.iter().position(|item| item.key == key)
    }

    pub fn is_empty(&self) -> bool {
        self.recent.is_empty()
    }
}

/// Selection commands remember an operation, never a previous track or form value.
pub fn key(command: &PaletteCommandKind) -> Option<String> {
    use PaletteCommandKind::*;
    Some(match command {
        OpenCommandGroup(_) => return None,
        FromTrackContext { kind, .. } => {
            use super::track_context::ContextKind as K;
            match kind {
                K::Separator => return None,
                K::PlayTrack => "PlayFocusedTrack".into(),
                K::PlayTrackAndFollowing => "PlayFocusedTrackAndFollowing".into(),
                K::PlayNextInQueue => "EnqueueSelectionNext".into(),
                K::AddToEndOfQueue => "EnqueueSelectionEnd".into(),
                K::OpenInLibrary => "OpenInLibrary".into(),
                K::ArtistBio { .. } => "ShowArtistBio".into(),
                K::SonicAdventure => "SonicAdventureFromFocusedTrack".into(),
                K::SearchExternal(target) => format!("Search{target:?}"),
                K::ShowDetails(destination) => format!("ShowDetails:{destination:?}"),
                _ => format!("track:{kind:?}"),
            }
        }
        JumpToArtist(_) => "JumpToArtist".into(),
        JumpToAlbum(_) => "JumpToAlbum".into(),
        BrowseStations { key, .. } => format!("BrowseStations:{key}"),
        Navidrome(super::sources::navidrome::commands::Command::ChoosePlaylist(
            super::sources::navidrome::playlist_edit::Addition::Tracks(_),
        )) => "AddSelectedTracksToPlaylist".into(),
        Navidrome(super::sources::navidrome::commands::Command::ChoosePlaylist(
            super::sources::navidrome::playlist_edit::Addition::Track(_),
        )) => "track:AddToPlaylist".into(),
        Navidrome(super::sources::navidrome::commands::Command::ChoosePlaylist(_)) => {
            "ChooseAlbumPlaylist".into()
        }
        Navidrome(super::sources::navidrome::commands::Command::EditEntry { edit, .. }) => {
            format!("PlaylistEntry:{edit:?}")
        }
        Navidrome(super::sources::navidrome::commands::Command::PromptNewPlaylist(None)) => {
            "NewPlaylist".into()
        }
        Navidrome(
            super::sources::navidrome::commands::Command::Append { .. }
            | super::sources::navidrome::commands::Command::PromptNewPlaylist(_)
            | super::sources::navidrome::commands::Command::CreatePlaylist { .. }
            | super::sources::navidrome::commands::Command::SetPlaylistName { .. },
        ) => {
            return None;
        }
        _ => format!("{command:?}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_configuration_preserves_palette_history() {
        let mut config = crate::config::Config::default();
        config.ui.command_history.remember("ToggleMute".into());
        config.ui.command_history.opened();
        crate::config::save_config(&config).unwrap();
        let restored = crate::config::load_config().unwrap();
        assert_eq!(restored.ui.command_history, config.ui.command_history);
    }

    #[test]
    fn recency_survives_restart_for_exactly_ten_openings_and_reuse_renews_it() {
        let mut history = History::default();
        history.remember("PlayPause".into());
        for _ in 0..10 {
            history = serde_json::from_str(&serde_json::to_string(&history).unwrap()).unwrap();
            history.opened();
            assert_eq!(history.rank("PlayPause"), Some(0));
        }
        history.opened();
        assert_eq!(history.rank("PlayPause"), None);
        history.remember("PlayPause".into());
        history.opened();
        history.remember("NextTrack".into());
        history.remember("PlayPause".into());
        assert_eq!(history.rank("PlayPause"), Some(0));
        assert_eq!(history.rank("NextTrack"), Some(1));
        for _ in 0..10 {
            history.opened();
        }
        assert_eq!(history.rank("PlayPause"), Some(0));
        history.opened();
        assert!(history.is_empty());
    }
    #[test]
    fn selection_identity_does_not_retain_track_metadata() {
        let command = PaletteCommandKind::JumpToArtist(Box::new(crate::library::models::Track {
            title: "Private title".into(),
            ..Default::default()
        }));
        assert_eq!(key(&command).as_deref(), Some("JumpToArtist"));
        assert_eq!(key(&PaletteCommandKind::OpenCommandGroup(vec![])), None);
    }
}
