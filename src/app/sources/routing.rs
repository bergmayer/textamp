//! Which actions must be consumed by a library provider before shared dispatch.
//!
//! Providers own I/O; shared handlers own reducers and navigation. Keep request
//! classifications exhaustive so new operations require an explicit routing
//! decision. Radio has a separate provider dispatcher. Account management is
//! independent of the active library.

use crate::app::action::*;

pub(super) fn requires_provider(action: &Action) -> bool {
    match action {
        Action::Data(action) => match action {
            DataAction::LoadInitialData
            | DataAction::LoadArtists
            | DataAction::LoadPlaylists
            | DataAction::LoadArtistAlbums
            | DataAction::LoadArtistAllTracks
            | DataAction::LoadSelectedAlbumTracks
            | DataAction::LoadAlbumTracks { .. }
            | DataAction::LoadCategoryTracks
            | DataAction::LoadSimilarAlbums { .. }
            | DataAction::LoadSimilarTracks { .. }
            | DataAction::LoadTrackPaneSimilar { .. }
            | DataAction::LoadSimilarArtists { .. }
            | DataAction::LoadRelated { .. } => true,
            DataAction::GoBackInRightPanel
            | DataAction::ListUp
            | DataAction::ListDown
            | DataAction::ListPageUp
            | DataAction::ListPageDown
            | DataAction::ListTop
            | DataAction::ListBottom => false,
        },
        Action::Miller(action) => match action {
            MillerAction::LoadArtistAlbumsForMiller { .. }
            | MillerAction::LoadAlbumTracksForMiller { .. }
            | MillerAction::LoadArtistAllTracksForMiller { .. }
            | MillerAction::LoadAllAlbumsForMiller { .. }
            | MillerAction::LoadGenreAlbumsForMiller { .. }
            | MillerAction::LoadGenreTracksForMiller { .. }
            | MillerAction::LoadPlaylistTracksForMiller { .. }
            | MillerAction::LoadMorePlaylistTracks { .. }
            | MillerAction::RefreshAlbumTracks { .. }
            | MillerAction::LoadAllLibraryTracksForMiller { .. } => true,
            MillerAction::ArtistAlbumsForMillerLoaded { .. }
            | MillerAction::AlbumTracksForMillerLoaded { .. }
            | MillerAction::ArtistAllTracksForMillerLoaded { .. }
            | MillerAction::GenreAlbumsForMillerLoaded { .. }
            | MillerAction::GenreTracksForMillerLoaded { .. }
            | MillerAction::AlbumTracksRefreshed { .. }
            | MillerAction::PlayTrackFromMiller { .. }
            | MillerAction::PlayGenreTrackFromMiller { .. }
            | MillerAction::PlayPlaylistTrackFromMiller { .. }
            // Compilation columns are built entirely from the shared catalog.
            | MillerAction::LoadCompilationsForMiller { .. }
            | MillerAction::LoadCompilationAlbumsForMiller { .. }
            | MillerAction::LoadCompilationAllTracksForMiller { .. }
            | MillerAction::LoadAllCompilationTracksForMiller { .. } => false,
        },
        Action::Browse(action) => match action {
            BrowseAction::LoadStations
            | BrowseAction::LoadTagList(_)
            | BrowseAction::LoadTagAlbums { .. } => true,
            BrowseAction::StationsLoaded { .. }
            | BrowseAction::TagListLoaded { .. }
            | BrowseAction::TagAlbumsLoaded { .. }
            | BrowseAction::RefreshTagView
            | BrowseAction::OpenTrackDetails
            | BrowseAction::CloseTrackDetails
            | BrowseAction::OpenInLibrary { .. } => false,
        },
        Action::Radio(_) => false, // Owned by sources::radio, never by a fallback.
        Action::Folders(action) => match action {
            FolderAction::LoadFolderRoot
            | FolderAction::NavigateIntoFolder { .. }
            | FolderAction::PlayFolderTracks
            | FolderAction::PlayFolderTrack { .. }
            | FolderAction::RefreshSubfolder(_) => true,
            FolderAction::FolderTracksLoaded { .. } | FolderAction::FolderTrackLoaded { .. } => {
                false
            }
        },
        Action::System(action) => matches!(
            action,
            SystemAction::RefreshCategory(_)
                | SystemAction::CheckStaleness(_)
                | SystemAction::LoadArtwork
                | SystemAction::LoadAlbumArt(_)
        ),
        Action::Queue(action) => matches!(
            action,
            QueueAction::PlayAlbum { .. }
                | QueueAction::PlayAlbumNow { .. }
                | QueueAction::PlayArtistTracks { .. }
                | QueueAction::PlayPlaylistNow { .. }
                | QueueAction::EnqueueAlbum { .. }
                | QueueAction::EnqueueAlbumNext { .. }
                | QueueAction::EnqueueArtistTracks { .. }
                | QueueAction::EnqueueArtistTracksNext { .. }
                | QueueAction::SaveQueueAsPlaylist(_)
        ),
        Action::Search(action) => matches!(
            action,
            SearchAction::AdventureLauncherDrillArtist { .. }
                | SearchAction::AdventureLauncherDrillAlbum { .. }
                | SearchAction::AdventureLauncherGenerate
                | SearchAction::ArtistRadioPickerLaunch
        ),
        Action::Settings(action) => matches!(action, SettingsAction::SetAdventureLength(_)),
        Action::Playback(_) => false,
        Action::Navigation(_) | Action::Source(_) => false,
    }
}

/// None means shared dispatch is safe; an unhandled request is a routing bug,
/// not permission to silently discard an unsupported operation.
pub(super) fn navidrome_fallback(action: &Action) -> Option<Vec<Action>> {
    requires_provider(action).then(|| {
        vec![
            SystemAction::ShowError("This library operation has no Navidrome handler".into())
                .into(),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_provider_handler_is_an_error_not_a_plex_fallback() {
        assert!(matches!(
            navidrome_fallback(&DataAction::LoadArtists.into()).as_deref(),
            Some([Action::System(SystemAction::ShowError(_))])
        ));
        assert!(requires_provider(
            &QueueAction::EnqueueAlbumNext {
                rating_key: "nav:album".into(),
                title: "Album".into()
            }
            .into()
        ));
    }

    #[test]
    fn shared_reducers_and_account_management_do_not_require_the_active_provider() {
        for action in [
            QueueAction::TracksLoaded {
                intent: QueueLoadIntent::Append {
                    label: "Album".into(),
                },
                result: Ok(vec![]),
            }
            .into(),
            QueueAction::ClearQueue.into(),
            PlaybackAction::Seek(10).into(),
            MillerAction::LoadCompilationsForMiller {
                replace_child: false,
            }
            .into(),
        ] {
            assert!(navidrome_fallback(&action).is_none(), "{action:?}");
        }
    }
}
