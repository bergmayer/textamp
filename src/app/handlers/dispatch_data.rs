//! Data loading dispatch handlers: LoadInitialData, LoadLibraries, LoadArtists, LoadAlbums,
//! LoadPlaylists, LoadArtistAlbums, LoadArtistAllTracks, LoadSelectedAlbumTracks,
//! LoadAlbumTracks, LoadCategoryTracks, GoBackInRightPanel, LoadSimilarAlbums,
//! LoadSimilarTracks, ListUp/Down/PageUp/PageDown/Top/Bottom.

use crate::app::action::DataAction;
use crate::app::state::RightPanelMode;
use crate::app::{Action, AppState, Event};
use crate::config::Config;

use anyhow::Result;
use tokio::sync::mpsc;

use super::helpers;

/// Dispatch data-loading actions. Returns follow-up actions.
pub async fn dispatch(
    _event_tx: &mpsc::Sender<Event>,
    _config: &Config,
    action: DataAction,
    state: &mut AppState,
) -> Result<Vec<Action>> {
    match action {
        DataAction::GoBackInRightPanel => {
            // Go from tracks back to albums view (for artist drill-down)
            if state.library.right_panel_mode == RightPanelMode::AlbumTracks {
                state.library.right_panel_mode = RightPanelMode::ArtistAlbums;
                state.library.selected_album_tracks.clear();
            }
        }

        DataAction::ListUp => {
            helpers::adjust_list_index(state, -1);
        }
        DataAction::ListDown => {
            helpers::adjust_list_index(state, 1);
            // Lazy load more if needed
        }
        DataAction::ListPageUp => {
            helpers::adjust_list_index(state, -10);
        }
        DataAction::ListPageDown => {
            helpers::adjust_list_index(state, 10);
        }
        DataAction::ListTop => {
            helpers::set_list_index(state, 0);
        }
        DataAction::ListBottom => {
            helpers::set_list_index(state, isize::MAX);
        }
        _ => anyhow::bail!("Unsupported library operation reached shared handler"),
    }
    Ok(vec![])
}
