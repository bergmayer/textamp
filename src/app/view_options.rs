//! Shared View Options labels, control semantics and checked state.
use super::{AppState, state::*};

/// Mutually exclusive sorting choices and independent view switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Radio,
    Checkbox,
}

/// One option retains its semantic identity across frontend layouts.
pub struct Choice {
    pub id: SortPopupOption,
    pub label: &'static str,
    pub kind: Kind,
    pub checked: bool,
}

pub fn for_column(state: &AppState, index: usize) -> Option<SortPopupState> {
    let column = state.browse_nav()?.columns.get(index)?;
    let mut popup = SortPopupState::new(
        index,
        column.title.clone(),
        state.column_options_type(index)?,
        column.sort_mode,
        column.artwork_visible,
        state.browse_category == BrowseCategory::Playlists,
    );
    if super::downloads::is_root_column(state, index) {
        popup.is_downloads = true;
        popup.rebuild_options(column.sort_mode);
    }
    Some(popup)
}

/// Project the same choices and ordering for terminal and native View Options.
pub fn choices(state: &AppState, popup: &SortPopupState) -> Vec<Choice> {
    let column = state
        .browse_nav()
        .and_then(|nav| nav.columns.get(popup.column_idx));
    popup
        .options
        .iter()
        .copied()
        .map(|id| {
            let label = match id {
                SortPopupOption::SortMode(mode) => match mode {
                    ColumnSortMode::Default => popup.default_label,
                    ColumnSortMode::ByArtist => "By artist",
                    ColumnSortMode::ByAlbum => "By album",
                    ColumnSortMode::ByTitle => "By title",
                    ColumnSortMode::ByDuration => "By duration",
                    ColumnSortMode::ByDateAdded => "Date added",
                    ColumnSortMode::Shuffled => "Shuffled",
                },
                SortPopupOption::Direction => "Reverse order",
                SortPopupOption::Artwork => "Show artwork",
                SortPopupOption::LargeArtwork => "Large Artwork",
                SortPopupOption::GroupByAlbum => "Group by album",
                SortPopupOption::CompilationOnlyArtists => "Show compilation-only artists",
            };
            let kind = if matches!(id, SortPopupOption::SortMode(_)) {
                Kind::Radio
            } else {
                Kind::Checkbox
            };
            let checked = if popup.is_downloads && id == SortPopupOption::GroupByAlbum {
                state.downloads.group_by_album
            } else {
                is_checked(state, column, id)
            };
            Choice {
                id,
                label,
                kind,
                checked,
            }
        })
        .collect()
}

/// Checked state shared by a column popover and the native View menu.
pub fn is_checked(state: &AppState, column: Option<&BrowseColumn>, id: SortPopupOption) -> bool {
    match id {
        SortPopupOption::CompilationOnlyArtists => state.show_compilation_only_artists,
        SortPopupOption::LargeArtwork => state.large_artwork,
        _ => column.is_some_and(|column| match id {
            SortPopupOption::SortMode(mode) => column.sort_mode == mode,
            SortPopupOption::Direction => {
                if column.sort_mode == ColumnSortMode::ByDateAdded {
                    column.sort_ascending
                } else {
                    !column.sort_ascending
                }
            }
            SortPopupOption::Artwork => column.artwork_visible,
            SortPopupOption::GroupByAlbum => column.grouped_by_album,
            SortPopupOption::CompilationOnlyArtists | SortPopupOption::LargeArtwork => false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_artwork_is_shared_and_keyboard_activation_dispatches_its_setting() {
        let mut state = AppState::new();
        let mut popup = SortPopupState::new(
            0,
            "Albums".into(),
            SortColumnType::Album,
            ColumnSortMode::Default,
            true,
            false,
        );
        popup.selected_index = popup
            .options
            .iter()
            .position(|option| *option == SortPopupOption::LargeArtwork)
            .unwrap();
        assert!(
            !choices(&state, &popup)
                .iter()
                .find(|choice| choice.id == SortPopupOption::LargeArtwork)
                .unwrap()
                .checked
        );
        state.large_artwork = true;
        assert!(
            choices(&state, &popup)
                .iter()
                .find(|choice| choice.id == SortPopupOption::LargeArtwork)
                .unwrap()
                .checked
        );
        state.popups.set_sort(Some(popup));
        let actions = crate::app::handlers::key_input::handle_key(
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ),
            &mut state,
            &crate::config::Config::default(),
        );
        assert!(matches!(
            actions.as_slice(),
            [crate::app::Action::Settings(
                crate::app::action::SettingsAction::SetLargeArtwork(false)
            )]
        ));
    }
}

/// Caption placement for album artwork at each view size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtworkLayout {
    Beside,
    Below,
}
impl From<bool> for ArtworkLayout {
    fn from(large: bool) -> Self {
        if large { Self::Below } else { Self::Beside }
    }
}

/// Album caption shared by library presentations, omitting an unknown year.
pub fn album_caption(title: &str, year: Option<u16>) -> std::borrow::Cow<'_, str> {
    match year {
        Some(year) => format!("{title} ({year})").into(),
        None => title.into(),
    }
}
