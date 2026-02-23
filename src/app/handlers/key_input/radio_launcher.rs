//! Radio launcher popup key handling.

use crossterm::event::{self, KeyCode};

use crate::app::Action;
use crate::app::state::{RadioLauncherTab, SearchFocus};
use crate::app::AppState;

/// Handle radio launcher popup keys.
pub(super) fn handle_radio_launcher_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    let launcher = match state.popups.radio_launcher.as_mut() {
        Some(l) => l,
        None => return vec![],
    };

    match key.code {
        KeyCode::Esc => {
            vec![Action::CloseRadioLauncher]
        }
        KeyCode::Enter => {
            match launcher.focus {
                SearchFocus::Input => {
                    // Move focus to results if we have any
                    if let Some(ref results) = launcher.results {
                        let count = result_count_for_tab(results, launcher.tab);
                        if count > 0 {
                            launcher.focus = SearchFocus::Results;
                            launcher.item_index = 0;
                        }
                    }
                    vec![]
                }
                SearchFocus::Results => {
                    // Select the highlighted result — start radio
                    vec![Action::RadioLauncherSelectResult]
                }
            }
        }
        KeyCode::Down => {
            match launcher.focus {
                SearchFocus::Input => {
                    if let Some(ref results) = launcher.results {
                        let count = result_count_for_tab(results, launcher.tab);
                        if count > 0 {
                            launcher.focus = SearchFocus::Results;
                            launcher.item_index = 0;
                        }
                    }
                    vec![]
                }
                SearchFocus::Results => {
                    if let Some(ref results) = launcher.results {
                        let total = result_count_for_tab(results, launcher.tab);
                        if total > 0 && launcher.item_index + 1 < total {
                            launcher.item_index += 1;
                        }
                    }
                    vec![]
                }
            }
        }
        KeyCode::Up => {
            match launcher.focus {
                SearchFocus::Input => vec![],
                SearchFocus::Results => {
                    if launcher.item_index == 0 {
                        launcher.focus = SearchFocus::Input;
                    } else {
                        launcher.item_index -= 1;
                    }
                    vec![]
                }
            }
        }
        KeyCode::Tab => {
            launcher.tab = launcher.tab.next();
            launcher.focus = SearchFocus::Input;
            launcher.item_index = 0;
            vec![]
        }
        KeyCode::BackTab => {
            launcher.tab = launcher.tab.prev();
            launcher.focus = SearchFocus::Input;
            launcher.item_index = 0;
            vec![]
        }
        KeyCode::Backspace => {
            launcher.query.pop();
            launcher.focus = SearchFocus::Input;
            launcher.item_index = 0;
            if !launcher.query.is_empty() {
                vec![Action::RadioLauncherSearch]
            } else {
                launcher.results = None;
                vec![]
            }
        }
        KeyCode::Char(c) => {
            launcher.query.push(c);
            launcher.focus = SearchFocus::Input;
            launcher.item_index = 0;
            vec![Action::RadioLauncherSearch]
        }
        _ => vec![],
    }
}

/// Count total selectable items for the given tab in radio launcher results.
fn result_count_for_tab(results: &crate::plex::models::SearchResults, tab: RadioLauncherTab) -> usize {
    match tab {
        RadioLauncherTab::All => results.artists.len(),
        RadioLauncherTab::Artists => results.artists.len(),
    }
}
