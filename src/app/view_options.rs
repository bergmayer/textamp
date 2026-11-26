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

/// Terminal controls open directly at the same path as the native View menu.
pub fn open_for_input(state: &mut AppState) -> Vec<super::Action> {
    if state.desktop_frontend {
        return vec![super::action::SearchAction::OpenSortPopup.into()];
    }
    state.palette.player_context = false;
    super::command_palette::run(
        PaletteCommandKind::OpenCommandGroup(vec!["View".into(), "View Options".into()]),
        state,
    )
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

pub(crate) fn command_option(command: &PaletteCommandKind) -> Option<SortPopupOption> {
    use PaletteCommandKind::*;
    Some(match command {
        ApplySort(mode) => SortPopupOption::SortMode(*mode),
        SetAlbumLayout(true) => SortPopupOption::AlbumGrid,
        SetAlbumLayout(false) => SortPopupOption::AlbumList,
        ReverseSort => SortPopupOption::Direction,
        ToggleArtwork => SortPopupOption::Artwork,
        ToggleGroupByAlbum => SortPopupOption::GroupByAlbum,
        ToggleCompilationArtists => SortPopupOption::CompilationOnlyArtists,
        _ => return None,
    })
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
                SortPopupOption::AlbumGrid => "Grid",
                SortPopupOption::AlbumList => "List",
                SortPopupOption::GroupByAlbum => "Group by album",
                SortPopupOption::CompilationOnlyArtists => "Show compilation-only artists",
            };
            let kind = if matches!(
                id,
                SortPopupOption::SortMode(_)
                    | SortPopupOption::AlbumGrid
                    | SortPopupOption::AlbumList
            ) {
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
        SortPopupOption::AlbumGrid => state.album_grid,
        SortPopupOption::AlbumList => !state.album_grid,
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
            SortPopupOption::CompilationOnlyArtists
            | SortPopupOption::AlbumList
            | SortPopupOption::AlbumGrid => false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_album_display_defaults_to_grid_and_an_explicit_list_round_trips() {
        use crate::config::{AlbumLayout, Config};
        let mut config: Config =
            toml::from_str("[ui]\nalbum_grid = false\ndesktop_large_album_covers = true").unwrap();
        assert_eq!(config.ui.album_layout, AlbumLayout::Grid);
        config.ui.album_layout = AlbumLayout::List;
        let saved = toml::to_string(&config).unwrap();
        assert_eq!(
            toml::from_str::<Config>(&saved).unwrap().ui.album_layout,
            AlbumLayout::List
        );
    }

    #[test]
    fn album_track_order_survives_direct_sort_commands() {
        use crate::app::handlers::key_input::sort_popup::{apply_sort_mode, toggle_sort_direction};
        let mut state = AppState::new();
        let tracks = ["Overture", "Finale"]
            .map(|title| crate::library::models::Track {
                title: title.into(),
                rating_key: title.into(),
                ..Default::default()
            })
            .to_vec();
        let mut column =
            BrowseColumn::new_with_tracks("Tracks", BrowseItem::from_tracks(&tracks), tracks);
        column.play_all_row = Some(PlayAllRow::Album {
            rating_key: "album".into(),
            title: "Album".into(),
        });
        state.artist_nav.columns = vec![column];
        assert!(for_column(&state, 0).is_none());
        apply_sort_mode(&mut state, 0, ColumnSortMode::ByTitle);
        toggle_sort_direction(&mut state, 0);
        let column = &state.artist_nav.columns[0];
        assert_eq!(column.sort_mode, ColumnSortMode::Default);
        assert!(column.sort_ascending);
        assert_eq!(column.tracks[0].title, "Overture");
        assert_eq!(column.items[1].title(), "Finale");
    }

    #[test]
    fn selecting_list_switches_the_default_album_grid() {
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
            .position(|option| *option == SortPopupOption::AlbumList)
            .unwrap();
        assert!(
            !choices(&state, &popup)
                .iter()
                .find(|choice| choice.id == SortPopupOption::AlbumList)
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
                crate::app::action::SettingsAction::SetAlbumGrid(false)
            )]
        ));
    }
}

/// Display positions before the first cover are full-width navigation rows.
#[derive(Clone, Copy)]
pub struct AlbumGrid {
    pub first: usize,
    pub columns: usize,
}
impl AlbumGrid {
    /// Adjacent tile in the same row, or the closest tile in the next row.
    pub fn neighbor(
        self,
        position: usize,
        count: usize,
        horizontal: i8,
        vertical: i8,
    ) -> Option<usize> {
        if position >= count {
            return None;
        }
        let band = self.band(position);
        if horizontal != 0 {
            let next = position.checked_add_signed(isize::from(horizontal))?;
            return (next < count && self.band(next) == band).then_some(next);
        }
        let next = band.checked_add_signed(isize::from(vertical))?;
        if next > self.band(count - 1) {
            return None;
        }
        Some(
            (self.start(next) + (position - self.start(band)))
                .min(self.start(next + 1) - 1)
                .min(count - 1),
        )
    }
    pub fn band(self, position: usize) -> usize {
        position.min(self.first) + position.saturating_sub(self.first) / self.columns
    }
    pub fn start(self, band: usize) -> usize {
        band.min(self.first) + band.saturating_sub(self.first) * self.columns
    }
}
pub fn album_grid(state: &AppState, index: usize) -> bool {
    state.album_grid
        && matches!(
            state.column_options_type(index),
            Some(SortColumnType::Album | SortColumnType::RecentAlbums)
        )
}
/// Album caption shared by library presentations, omitting an unknown year.
pub fn album_caption(title: &str, year: Option<u16>) -> std::borrow::Cow<'_, str> {
    match year {
        Some(year) => format!("{title} ({year})").into(),
        None => title.into(),
    }
}

/// Split at the word boundary nearest the typographic midpoint. Renderers keep
/// a single line whenever it fits and use normal wrapping for longer captions.
pub fn caption_lines(caption: &str) -> Option<(&str, &str)> {
    use unicode_width::UnicodeWidthStr;
    let width = caption.width();
    let mut left = 0;
    let mut best = None;
    for (index, ch) in caption.char_indices() {
        if ch.is_whitespace() && left > 0 {
            let right = width.saturating_sub(left + 1);
            if right != 0 && best.is_none_or(|(_, score)| left.abs_diff(right) < score) {
                best = Some((index, left.abs_diff(right)));
            }
        }
        left += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    best.map(|(i, _)| (caption[..i].trim_end(), caption[i..].trim_start()))
}

pub fn wrapped_caption(caption: &str, width: usize) -> std::borrow::Cow<'_, str> {
    use unicode_width::UnicodeWidthStr;
    if caption.width() > width
        && let Some((a, b)) = caption_lines(caption)
        && a.width().max(b.width()) <= width
    {
        return format!("{a}\n{b}").into();
    }
    caption.into()
}

pub fn terminal_grid_columns(width: u16) -> usize {
    usize::from((width / 24).clamp(if width >= 12 { 2 } else { 1 }, 8))
}
