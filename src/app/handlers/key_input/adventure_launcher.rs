//! Keyboard and pointer activation for the single Journey form.
use crate::app::{
    Action, AppState,
    action::*,
    journey::{self, JourneyControl as Control},
    state::*,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Focus and activate a Journey field or command through shared actions.
pub(in crate::app::handlers) fn activate(control: Control, state: &mut AppState) -> Vec<Action> {
    let Some(launcher) = state.popups.adventure_launcher_mut() else {
        return vec![];
    };
    if matches!(launcher.stage, JourneyStage::Generating { .. }) && control != Control::Cancel {
        return vec![];
    }
    launcher.terminal_focus = control;
    match control {
        Control::Start | Control::End => {
            let step = if control == Control::Start {
                AdventureStep::FindStartTrack
            } else {
                AdventureStep::FindEndTrack
            };
            let edit = !matches!(launcher.stage, JourneyStage::Choosing);
            let mut actions = Vec::new();
            if edit {
                actions.push(SearchAction::AdventureLauncherEdit.into());
            }
            actions.push(SearchAction::AdventureLauncherSetStep(step).into());
            actions
        }
        Control::Swap => vec![
            SearchAction::AdventureLauncherEdit.into(),
            SearchAction::AdventureLauncherReverse.into(),
        ],
        Control::Cancel => vec![SearchAction::CloseAdventureLauncher.into()],
        Control::Preview => vec![
            match launcher.stage {
                JourneyStage::Ready(_) => SearchAction::AdventureLauncherPlay,
                _ if launcher.purpose != TrackPickerPurpose::Journey => {
                    SearchAction::StartPickedTrackRadio
                }
                _ => SearchAction::AdventureLauncherGenerate,
            }
            .into(),
        ],
        Control::Results => {
            launcher.focus = SearchFocus::Results;
            match launcher.stage {
                JourneyStage::Ready(_) => vec![],
                _ => journey::select(launcher),
            }
        }
        Control::Count if !matches!(launcher.stage, JourneyStage::Choosing) => {
            vec![SearchAction::AdventureLauncherEdit.into()]
        }
        Control::Count | Control::Types => vec![],
    }
}

/// Every form field, result and command is reachable with Tab and arrows.
pub(super) fn handle_adventure_launcher_keys(key: KeyEvent, state: &mut AppState) -> Vec<Action> {
    let desktop = state.desktop_frontend;
    let Some(launcher) = state.popups.adventure_launcher_mut() else {
        return vec![];
    };
    if desktop
        && launcher.focus == SearchFocus::Results
        && key.code == KeyCode::Enter
        && matches!(launcher.stage, JourneyStage::Choosing)
    {
        return journey::select(launcher);
    }
    if key.code == KeyCode::Esc {
        return vec![
            if matches!(
                launcher.stage,
                JourneyStage::Ready(_) | JourneyStage::Failed(_)
            ) {
                SearchAction::AdventureLauncherEdit
            } else {
                SearchAction::CloseAdventureLauncher
            }
            .into(),
        ];
    }
    if matches!(launcher.stage, JourneyStage::Generating { .. }) {
        return vec![];
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Enter {
        return activate(Control::Preview, state);
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        let control = match key.code {
            KeyCode::Char('s') => Some(Control::Start),
            KeyCode::Char('e') => Some(Control::End),
            KeyCode::Char('m') => Some(Control::Count),
            KeyCode::Char('p') => Some(Control::Preview),
            KeyCode::Char('w') => Some(Control::Swap),
            _ => None,
        };
        return control
            .map(|control| activate(control, state))
            .unwrap_or_default();
    }
    if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
        let controls = journey::controls(launcher.purpose);
        let index = controls
            .iter()
            .position(|control| *control == launcher.terminal_focus)
            .unwrap_or(0);
        let next = if key.code == KeyCode::Tab {
            (index + 1) % controls.len()
        } else {
            (index + controls.len() - 1) % controls.len()
        };
        let control = controls[next];
        launcher.terminal_focus = control;
        if matches!(control, Control::Start | Control::End | Control::Count) {
            return activate(control, state);
        }
        return vec![];
    }
    if desktop && launcher.focus == SearchFocus::Results {
        launcher.terminal_focus = Control::Results;
    }
    let control = launcher.terminal_focus;
    if matches!(control, Control::Start | Control::End | Control::Count) {
        let step = if control == Control::Start {
            AdventureStep::FindStartTrack
        } else {
            AdventureStep::FindEndTrack
        };
        match key.code {
            KeyCode::Enter | KeyCode::Down => {
                launcher.terminal_focus = Control::Results;
                launcher.focus = SearchFocus::Results;
                return vec![];
            }
            KeyCode::Char(_) | KeyCode::Backspace => {
                let selected = if step == AdventureStep::FindStartTrack {
                    &launcher.start_track
                } else {
                    &launcher.end_track
                };
                let mut query = if launcher.step == step && !launcher.query.is_empty() {
                    launcher.query.clone()
                } else if key.code == KeyCode::Backspace {
                    selected
                        .as_ref()
                        .map(|track| track.title.clone())
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                match key.code {
                    KeyCode::Char(character) => query.push(character),
                    KeyCode::Backspace => {
                        query.pop();
                    }
                    _ => {}
                }
                launcher.focus = SearchFocus::Input;
                return vec![
                    SearchAction::AdventureLauncherSetEndpointQuery { step, query }.into(),
                ];
            }
            _ => return vec![],
        }
    }
    if control == Control::Count {
        let mut value = launcher.track_count_input.clone();
        match key.code {
            KeyCode::Char(c) if c.is_ascii_digit() => value.push(c),
            KeyCode::Backspace => {
                value.pop();
            }
            KeyCode::Enter => {
                launcher.terminal_focus = Control::Preview;
                return vec![];
            }
            _ => return vec![],
        }
        return vec![SearchAction::AdventureLauncherSetTrackCount(value).into()];
    }
    if control == Control::Types && matches!(key.code, KeyCode::Left | KeyCode::Right) {
        let tabs = [
            SearchTab::Global,
            SearchTab::Artists,
            SearchTab::Albums,
            SearchTab::Tracks,
        ];
        let index = tabs
            .iter()
            .position(|tab| *tab == launcher.search_tab)
            .unwrap_or(0);
        launcher.search_tab = tabs[if key.code == KeyCode::Right {
            (index + 1) % tabs.len()
        } else {
            (index + tabs.len() - 1) % tabs.len()
        }];
        launcher.item_index = 0;
        launcher.scroll_pin = None;
        return vec![];
    }
    if control == Control::Results {
        let count = if let JourneyStage::Ready(tracks) = &launcher.stage {
            tracks.len()
        } else {
            journey::choices(launcher).len()
        };
        launcher.scroll_pin = None;
        match key.code {
            KeyCode::Up if launcher.item_index == 0 => {
                launcher.terminal_focus = if launcher.step == AdventureStep::FindStartTrack {
                    Control::Start
                } else {
                    Control::End
                };
                launcher.focus = SearchFocus::Input;
            }
            KeyCode::Up => launcher.item_index = launcher.item_index.saturating_sub(1),
            KeyCode::Down => {
                launcher.item_index = (launcher.item_index + 1).min(count.saturating_sub(1))
            }
            KeyCode::Home => launcher.item_index = 0,
            KeyCode::End => launcher.item_index = count.saturating_sub(1),
            KeyCode::PageUp => launcher.item_index = launcher.item_index.saturating_sub(8),
            KeyCode::PageDown => {
                launcher.item_index = (launcher.item_index + 8).min(count.saturating_sub(1))
            }
            KeyCode::Left if !matches!(launcher.drill, AdventureDrillLevel::Search) => {
                return vec![SearchAction::AdventureLauncherBack.into()];
            }
            KeyCode::Enter if matches!(launcher.stage, JourneyStage::Ready(_)) => {
                launcher.terminal_focus = Control::Preview;
            }
            KeyCode::Enter => return journey::select(launcher),
            _ => {}
        }
        return vec![];
    }
    if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) {
        return activate(control, state);
    }
    vec![]
}
