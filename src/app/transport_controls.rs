//! Terminal transport focus projected onto shared playback and navigation commands.
use super::{Action, AppState, action::*, state::*};

/// Controls in visual keyboard order, independent of viewport geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportControl {
    Sidebar,
    Queue,
    Previous,
    Play,
    Next,
    Track,
    Artist,
    Album,
    Seek,
    Menu,
    Lyrics,
    Mute,
    Volume,
    Clear,
    Player,
}
impl TransportControl {
    pub fn enabled(self, state: &AppState) -> bool {
        match self {
            Self::Track | Self::Menu | Self::Lyrics => state.current_track().is_some(),
            Self::Seek => {
                state.playback.duration_ms > 0 && state.playback.status != PlayStatus::Stopped
            }
            Self::Clear => {
                (state.tui_queue_visible || state.view == View::Queue)
                    && super::queue_controls::can_clear_tab(state)
            }
            _ => true,
        }
    }
    /// Keyboard traversal order, matching the visual control groups.
    pub const ALL: &'static [Self] = &[
        #[cfg(target_os = "macos")]
        Self::Sidebar,
        Self::Previous,
        Self::Play,
        Self::Next,
        Self::Track,
        Self::Artist,
        Self::Album,
        Self::Seek,
        Self::Menu,
        Self::Lyrics,
        Self::Mute,
        Self::Volume,
        Self::Clear,
        Self::Player,
        #[cfg(target_os = "macos")]
        Self::Queue,
    ];
    /// Accessible description shown while the control has terminal focus.
    pub fn label(self) -> &'static str {
        match self {
            Self::Sidebar => "Show / Hide Sidebar",
            Self::Queue => "Show / Hide Queue",
            Self::Previous => "Previous track",
            Self::Play => "Play / Pause",
            Self::Next => "Next track",
            Self::Track => "Show Track Details",
            Self::Artist => "Show Artist Details",
            Self::Album => "Show Album Details",
            Self::Seek => "Track position: Left / Right",
            Self::Menu => "Track commands",
            Self::Lyrics => "Lyrics",
            Self::Clear => "Clear current queue tab",
            Self::Mute => "Mute",
            Self::Volume => "Volume: Up / Down",
            Self::Player => "Library / Now Playing",
        }
    }
}

/// Activate the same music operation used by either frontend's transport.
pub fn activate(state: &mut AppState, control: TransportControl) -> Vec<Action> {
    use TransportControl::*;
    if !control.enabled(state) {
        return vec![];
    }
    match control {
        Sidebar => vec![NavigationAction::ToggleSidebar.into()],
        Queue => vec![NavigationAction::ToggleQueuePanel.into()],
        Previous => vec![PlaybackAction::Previous.into()],
        Play => vec![PlaybackAction::TogglePlayPause.into()],
        Next => vec![PlaybackAction::Next.into()],
        Lyrics if !state.tui_player_raised() => vec![
            NavigationAction::SetView(View::NowPlaying).into(),
            PlaybackAction::ShowLyrics.into(),
        ],
        Lyrics => vec![PlaybackAction::ShowLyrics.into()],
        Mute => vec![PlaybackAction::ToggleMute.into()],
        Clear => vec![super::queue_controls::clear_tab(state.showing_queue_history).into()],
        Volume | Seek => vec![],
        Menu => {
            state.palette.player_context = true;
            super::command_palette::open(state)
        }
        Track | Artist | Album => state
            .current_track()
            .map(|track| {
                BrowseAction::OpenDetailsInLibrary {
                    track: Box::new(track.clone()),
                    destination: match control {
                        Artist => super::library_details::Destination::Artist,
                        Album => super::library_details::Destination::Album,
                        _ => super::library_details::Destination::Track,
                    },
                }
                .into()
            })
            .into_iter()
            .collect(),
        Player => vec![
            NavigationAction::SetView(
                if state.tui_player_raised()
                    || (state.desktop_frontend
                        && state.windowed_presentation.workspace()
                            == Some(super::windowed_presentation::Workspace::NowPlaying))
                    || (state.view == View::Queue
                        && state.tui_queue_workspace == TuiWorkspace::NowPlaying)
                {
                    View::Browse
                } else {
                    View::NowPlaying
                },
            )
            .into(),
            SystemAction::LoadWaveform.into(),
        ],
    }
}

/// Rendered controls in keyboard order; compact layouts skip hidden targets.
pub fn visible(state: &AppState) -> Vec<TransportControl> {
    state.hit_regions.transport.as_ref().map_or_else(
        || TransportControl::ALL.to_vec(),
        |region| {
            region
                .controls
                .iter()
                .map(|(_, control)| *control)
                .filter(|control| control.enabled(state))
                .collect()
        },
    )
}
