//! Shared playback command definitions; frontends choose their placement and gestures.
use super::{Action, AppState, action::*, state::PlayAllRow};
use crate::library::models::Track;
/// A command and its semantic variants, independent of control placement.
#[derive(Clone)]
pub struct Control {
    pub label: String,
    /// A compact provider mark, retaining the full accessible label.
    pub mark: Option<&'static str>,
    pub actions: std::sync::Arc<Vec<Action>>,
    pub alternate: Option<(String, std::sync::Arc<Vec<Action>>)>,
    next: Option<(String, std::sync::Arc<Vec<Action>>)>,
    next_following: Option<(String, std::sync::Arc<Vec<Action>>)>,
}
impl Control {
    /// Construct a command with its primary effect.
    pub fn new(label: impl Into<String>, action: impl Into<Action>) -> Self {
        Self {
            label: label.into(),
            mark: None,
            actions: std::sync::Arc::new(vec![action.into()]),
            alternate: None,
            next: None,
            next_following: None,
        }
    }
    /// Attach the selected-track-and-following variant.
    pub fn with_following(mut self, label: &str, action: impl Into<Action>) -> Self {
        self.alternate = Some((label.into(), std::sync::Arc::new(vec![action.into()])));
        self
    }
    /// Attach queue-next variants to a compact primary control.
    pub fn with_next_control(mut self, next: &Self) -> Self {
        self.next = Some((next.label.clone(), next.actions.clone()));
        self.next_following = next.alternate.clone();
        self
    }
    /// Whether the primary command has a separate queue-next variant.
    pub fn has_next(&self) -> bool {
        self.next.is_some()
    }
    /// Resolve a semantic command variant without interpreting physical keys.
    pub fn resolve(&self, variant: Variant) -> (&str, &[Action]) {
        let alternate = match variant {
            Variant::Primary => None,
            Variant::Next => self.next.as_ref(),
            Variant::Following => self.alternate.as_ref(),
            Variant::NextFollowing => self.next_following.as_ref().or(self.next.as_ref()),
        };
        alternate.map_or(
            (self.label.as_str(), self.actions.as_slice()),
            |(label, actions)| (label.as_str(), actions.as_slice()),
        )
    }
}

/// Semantic modifier choices shared by physical-key and pointer adapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Primary,
    Next,
    Following,
    NextFollowing,
}

/// Explicit Play, Play Next and Add commands in canonical order.
pub fn track_controls(track: &Track, following: Option<&[Track]>) -> Vec<Control> {
    DetailControl::ALL
        .into_iter()
        .map(|operation| {
            let control = Control::new(
                operation.label(),
                track_operation(track, following, TrackRange::Single, operation.placement()),
            );
            if following.is_some_and(|tracks| !tracks.is_empty()) {
                control.with_following(
                    operation.label_for(TrackRange::Following),
                    track_operation(
                        track,
                        following,
                        TrackRange::Following,
                        operation.placement(),
                    ),
                )
            } else {
                control
            }
        })
        .collect()
}
/// Append tracks through the source-independent queue reducer.
pub fn append_tracks(tracks: &[Track], label: &str) -> QueueAction {
    QueueAction::TracksLoaded {
        intent: QueueLoadIntent::Append {
            label: label.into(),
        },
        result: Ok(tracks.to_vec()),
    }
}
/// Album commands retain the album identity for lazy source loading.
pub fn album_controls(key: &str, title: &str) -> Vec<Control> {
    let mut controls = vec![
        Control::new(
            "Play Album",
            QueueAction::PlayAlbumNow {
                rating_key: key.into(),
                title: title.into(),
            },
        ),
        Control::new(
            "Add Album to Queue",
            QueueAction::EnqueueAlbum {
                rating_key: key.into(),
                title: title.into(),
            },
        ),
        Control::new(
            "Play Album Next",
            QueueAction::EnqueueAlbumNext {
                rating_key: key.into(),
                title: title.into(),
            },
        ),
    ];
    controls.swap(1, 2);
    controls
}
/// Commands for the selected collection and its ordered tracks.
pub fn collection_controls(row: &PlayAllRow, tracks: &[Track]) -> Vec<Control> {
    if let PlayAllRow::Album { rating_key, title } = row {
        return album_controls(rating_key, title);
    }
    let mut play = play_all(row, tracks);
    if matches!(row, PlayAllRow::ArtistRadio { .. }) {
        return vec![play];
    }
    play.label = if matches!(row, PlayAllRow::Playlist { .. }) {
        "Play Playlist"
    } else {
        "Play All Tracks"
    }
    .into();
    let next = Control::new("Play Next", QueueAction::EnqueueTracksNext(tracks.to_vec()));
    vec![
        play,
        next,
        Control::new("Add to Queue", append_tracks(tracks, "from collection")),
    ]
}

/// Primary collection command, including continuous artist radio.
pub fn play_all(row: &PlayAllRow, tracks: &[Track]) -> Control {
    match row {
        PlayAllRow::ArtistRadio {
            artist_key,
            artist_name,
        } => Control::new(
            "Artist Mix",
            crate::app::action::RadioAction::StartArtistRadio {
                key: artist_key.clone(),
                title: artist_name.clone(),
            },
        ),
        PlayAllRow::Album { rating_key, title } => Control::new(
            row.label(),
            QueueAction::PlayAlbumNow {
                rating_key: rating_key.clone(),
                title: title.clone(),
            },
        ),
        PlayAllRow::Playlist { rating_key, title } => Control::new(
            row.label(),
            QueueAction::PlayPlaylistNow {
                playlist_key: rating_key.clone(),
                title: title.clone(),
            },
        ),
        PlayAllRow::AllTracks { .. } => {
            Control::new(row.label(), QueueAction::PlayTracksNow(tracks.to_vec()))
        }
    }
}

/// Track range for an explicit playback or queue command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackRange {
    Single,
    Following,
}
/// Position at which a track operation takes effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueuePlacement {
    Now,
    Next,
    End,
}

/// Track header actions share the current ordered list and queue reducers.
pub fn track_action(
    state: &AppState,
    track: &Track,
    range: TrackRange,
    placement: QueuePlacement,
) -> Vec<Action> {
    let following = state
        .palette_track_list()
        .filter(|(tracks, index)| {
            tracks
                .get(*index)
                .is_some_and(|selected| selected.rating_key == track.rating_key)
        })
        .map(|(tracks, index)| super::downloads::ready_selection(state, &tracks[index..]));
    vec![track_operation(
        track,
        following.as_deref(),
        range,
        placement,
    )]
}

fn track_operation(
    track: &Track,
    following: Option<&[Track]>,
    range: TrackRange,
    placement: QueuePlacement,
) -> Action {
    if range == TrackRange::Single && placement == QueuePlacement::Now {
        return QueueAction::PlayTrack(Box::new(track.clone())).into();
    }
    let tracks = if range == TrackRange::Following {
        following
            .filter(|tracks| !tracks.is_empty())
            .unwrap_or(std::slice::from_ref(track))
    } else {
        std::slice::from_ref(track)
    };
    match placement {
        QueuePlacement::Now => QueueAction::PlayTracksNow(tracks.to_vec()),
        QueuePlacement::Next => QueueAction::EnqueueTracksNext(tracks.to_vec()),
        QueuePlacement::End => append_tracks(tracks, "from tracks"),
    }
    .into()
}

/// Playback operations in semantic order; adapters choose their placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailControl {
    Play,
    Next,
    Add,
}
impl DetailControl {
    pub const ALL: [Self; 3] = [Self::Play, Self::Next, Self::Add];
    pub const COUNT: usize = Self::ALL.len();

    /// Canonical action title.
    pub fn label(self) -> &'static str {
        match self {
            Self::Play => "Play Track",
            Self::Next => "Play Next",
            Self::Add => "Add to Queue",
        }
    }
    /// Accessible action name for the selected track range.
    pub fn label_for(self, range: TrackRange) -> &'static str {
        if range == TrackRange::Single {
            return self.label();
        }
        match self {
            Self::Play => "Play Track and Following",
            Self::Next => "Play Track and Following Next",
            Self::Add => "Add Track and Following to Queue",
        }
    }
    /// Queue destination for this command.
    pub fn placement(self) -> QueuePlacement {
        match self {
            Self::Play => QueuePlacement::Now,
            Self::Next => QueuePlacement::Next,
            Self::Add => QueuePlacement::End,
        }
    }
    /// Resolve an explicit track command for the requested semantic range.
    pub fn activate(
        self,
        state: &AppState,
        track: &crate::library::models::Track,
        range: TrackRange,
    ) -> Vec<Action> {
        track_action(state, track, range, self.placement())
    }
}
