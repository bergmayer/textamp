//! Queue-tool ordering, availability, state and commands shared by all frontends.
use super::{Action, AppState, action::*, presentation::NpSidebarButton as Tool, state::*};

/// A command's current presentation state, without geometry or widget types.
#[derive(Clone)]
pub struct Definition {
    pub tool: Tool,
    pub label: &'static str,
    pub group: &'static str,
    pub description: &'static str,
    pub active: bool,
    pub disabled_reason: Option<&'static str>,
}

/// Canonical menu order keeps destructive clearing below save and undo.
pub const ORDER: &[Tool] = &[
    Tool::Shuffle,
    Tool::Repeat,
    Tool::AutoMix,
    Tool::Twins,
    Tool::Twofer,
    Tool::Stretch,
    Tool::Doppelganger,
    Tool::OppositeDay,
    Tool::Enemies,
    Tool::Save,
    Tool::Undo,
    Tool::ClearQueue,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    QueueType,
    Tools,
}
impl Menu {
    pub fn label(self) -> &'static str {
        match self {
            Self::QueueType => "Queue Type",
            Self::Tools => "Queue Tools",
        }
    }
    pub fn open(self, state: &mut AppState) -> Vec<Action> {
        super::command_palette::run(
            PaletteCommandKind::OpenCommandGroup(vec!["Queue".into(), self.label().into()]),
            state,
        )
    }
}

/// Resolve capabilities and queue requirements against current state.
pub fn definition(state: &AppState, tool: Tool) -> Definition {
    let length = state.playback_tracks().len();
    let sonic = || super::sources::navidrome::recommendations::available(state);
    let (description, active) = match tool {
        Tool::Shuffle => (
            "Shuffle the queue",
            (state.queue.sort_mode == QueueSortMode::Shuffle && !state.queue.original.is_empty())
                || state
                    .queue
                    .shuffle_undo_queue
                    .as_ref()
                    .is_some_and(|tracks| !tracks.is_empty()),
        ),
        Tool::Repeat => (
            match state.queue.repeat {
                RepeatMode::Off => "Repeat Off",
                RepeatMode::Track => "Repeat Track",
                RepeatMode::Queue => "Repeat Queue",
            },
            state.queue.repeat != RepeatMode::Off,
        ),
        Tool::Twins => ("Add a sonic match after each track", false),
        Tool::Twofer => ("Pair tracks with another by the same artist", false),
        Tool::Stretch => ("Insert short bridges between tracks", false),
        Tool::Doppelganger => ("Replace tracks with sonic matches", false),
        Tool::OppositeDay => (
            "Replace tracks with the greatest average sonic distance from this queue",
            false,
        ),
        Tool::Enemies => ("Follow each track with a sonically distant track", false),
        Tool::ClearQueue => ("Clear the queue and stop the station", false),
        Tool::Save => ("Save the queue as a playlist", false),
        Tool::Undo => ("Undo the last queue edit", false),
        Tool::AutoMix => ("Smoothly overlap adjacent tracks", state.playback.auto_mix),
    };
    let disabled_reason = match tool {
        Tool::Shuffle | Tool::AutoMix if length < 2 && !active => {
            Some("Add at least two tracks to the queue")
        }
        Tool::Repeat if length == 0 && !active => Some("The queue is empty"),
        Tool::Stretch if length < 2 => Some("Add at least two tracks to the queue"),
        Tool::Twins
        | Tool::Twofer
        | Tool::Doppelganger
        | Tool::OppositeDay
        | Tool::Enemies
        | Tool::Save
            if length == 0 =>
        {
            Some("The queue is empty")
        }
        Tool::Twins
        | Tool::Stretch
        | Tool::Doppelganger
        | Tool::OppositeDay
        | Tool::Enemies
        | Tool::Save
            if state.downloads.is_offline() =>
        {
            Some("Unavailable offline")
        }
        Tool::Twofer if state.sources.active.navidrome().is_none() => {
            Some("Choose a server library to edit this queue")
        }
        Tool::Twins | Tool::Stretch | Tool::Doppelganger if !sonic() => {
            Some("Sonic analysis is unavailable for this library")
        }
        Tool::OppositeDay | Tool::Enemies
            if !sonic() || super::sources::audiomuse::connection(state).is_none() =>
        {
            Some("Connect AudioMuse to rank distant tracks")
        }
        Tool::Save
            if !state
                .sources
                .active
                .capabilities()
                .supports(crate::library::capabilities::Feature::SavePlaylist) =>
        {
            Some("This library cannot save playlists")
        }
        Tool::ClearQueue
            if length == 0
                && state.playback_mode != PlaybackMode::Radio
                && state.station_starting.is_none() =>
        {
            Some("The queue is empty")
        }
        Tool::Undo if state.queue.undo_snapshot.is_none() => Some("No queue edit to undo"),
        _ => None,
    };
    Definition {
        tool,
        label: tool.label(),
        group: match tool {
            Tool::Shuffle | Tool::Repeat | Tool::AutoMix => "Playback",
            Tool::Twins
            | Tool::Twofer
            | Tool::Stretch
            | Tool::Doppelganger
            | Tool::OppositeDay
            | Tool::Enemies => "Remix",
            Tool::Save | Tool::Undo | Tool::ClearQueue => "Queue",
        },
        description,
        active,
        disabled_reason,
    }
}

/// Available queue tools in the same order for rendering and keyboard navigation.
pub fn definitions(state: &AppState) -> Vec<Definition> {
    ORDER
        .iter()
        .copied()
        .map(|tool| definition(state, tool))
        .collect()
}

/// Execute a tool using fresh state, retaining shared playback and source reducers.
pub fn activate(state: &mut AppState, tool: Tool) -> Vec<Action> {
    if let Some(reason) = definition(state, tool).disabled_reason {
        state.set_status(reason.into());
        return vec![];
    }
    vec![match tool {
        Tool::AutoMix => PlaybackAction::ToggleAutoMix.into(),
        Tool::ClearQueue => QueueAction::ClearQueue.into(),
        Tool::Shuffle
            if state
                .queue
                .shuffle_undo_queue
                .as_ref()
                .is_some_and(|tracks| !tracks.is_empty()) =>
        {
            let mut actions = vec![QueueAction::RemixUndoShuffle.into()];
            if state.queue.sort_mode == QueueSortMode::Shuffle && !state.queue.original.is_empty() {
                actions.push(QueueAction::ToggleQueueShuffle.into());
            }
            return actions;
        }
        Tool::Shuffle => QueueAction::ToggleQueueShuffle.into(),
        Tool::Repeat if state.playback_tracks().is_empty() => {
            QueueAction::SetRepeat(RepeatMode::Off).into()
        }
        Tool::Repeat => QueueAction::SetRepeat(match state.queue.repeat {
            RepeatMode::Off => RepeatMode::Queue,
            RepeatMode::Queue => RepeatMode::Track,
            RepeatMode::Track => RepeatMode::Off,
        })
        .into(),
        Tool::Twins => QueueAction::RemixGemini.into(),
        Tool::Twofer => QueueAction::RemixTwofer.into(),
        Tool::Stretch => QueueAction::RemixStretch.into(),
        Tool::Doppelganger => QueueAction::RemixDoppelganger.into(),
        Tool::OppositeDay => QueueAction::RemixOppositeDay.into(),
        Tool::Enemies => QueueAction::RemixEnemies.into(),
        Tool::Save => QueueAction::PromptSavePlaylist.into(),
        Tool::Undo => QueueAction::UndoLastRemix.into(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_and_single_track_queues_disable_inapplicable_edits() {
        let mut state = AppState::new();
        assert!(
            definitions(&state)
                .iter()
                .all(|tool| tool.disabled_reason.is_some())
        );
        state.queue.tracks.push(Default::default());
        for tool in [Tool::Shuffle, Tool::AutoMix, Tool::Stretch, Tool::Twofer] {
            assert!(
                definition(&state, tool).disabled_reason.is_some(),
                "{tool:?}"
            );
            assert!(activate(&mut state, tool).is_empty());
        }
        assert!(definition(&state, Tool::Repeat).disabled_reason.is_none());
        assert!(
            definition(&state, Tool::ClearQueue)
                .disabled_reason
                .is_none()
        );
        crate::app::queue_edit::record(&mut state, "Clear Queue");
        state.queue.tracks.clear();
        assert!(definition(&state, Tool::Undo).disabled_reason.is_none());
    }

    #[test]
    fn tools_resolve_availability_and_continuous_state_from_live_state() {
        let mut state = AppState::new();
        assert!(definition(&state, Tool::Save).disabled_reason.is_some());
        assert!(activate(&mut state, Tool::Undo).is_empty());
        state.radio.active_station = Some(ActiveStation {
            source: crate::library::models::RadioSource::Related("seed".into()),
            title: "Continue with Related Tracks".into(),
        });
        state.playback_mode = PlaybackMode::Radio;
        state.radio.tracks = vec![Default::default(), Default::default()];
        state.playback.auto_mix = false;
        assert!(!definition(&state, Tool::AutoMix).active);
        assert!(matches!(
            activate(&mut state, Tool::AutoMix).as_slice(),
            [Action::Playback(PlaybackAction::ToggleAutoMix)]
        ));
        state.queue.repeat = RepeatMode::Queue;
        assert!(matches!(
            activate(&mut state, Tool::Repeat).as_slice(),
            [Action::Queue(QueueAction::SetRepeat(RepeatMode::Track))]
        ));
        let tools = definitions(&state);
        let trash = tools
            .iter()
            .position(|item| item.tool == Tool::ClearQueue)
            .unwrap();
        assert_eq!(trash, tools.len() - 1);
        assert_eq!(tools[trash - 1].tool, Tool::Undo);
        assert_eq!(tools[trash - 2].tool, Tool::Save);
    }
}
