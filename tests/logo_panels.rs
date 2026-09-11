use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, layout::Rect, Terminal};
use textamp::app::handlers::{key_input, mouse_input};
use textamp::app::state::{
    CategoryRow, MillerLayoutMode, SettingsFocus, SettingsSection, TextampSetting, View,
};
use textamp::app::theme::ThemeName;
use textamp::app::AppState;
use textamp::config::Config;
use textamp::ui::theme::Theme;

fn draw(state: &mut AppState, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, state)))
        .unwrap();
    terminal
}

#[test]
fn search_landing_replaces_content_without_receiving_hidden_content_input() {
    for layout in [MillerLayoutMode::Shrinking, MillerLayoutMode::Scrolling] {
        for tall in [false, true] {
            let mut state = AppState::new();
            assert!(matches!(
                state.category_rows()[state.category_column_index],
                CategoryRow::Category(_)
            ));
            state.view = View::Browse;
            state.category_column_focused = true;
            state.category_column_index = 0;
            state.miller_layout = layout;
            state.tall_mode = tall;
            let category = state.browse_category;
            let terminal = draw(&mut state, 150, 50);
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect();
            assert!(text.contains("press enter for search"));
            assert_eq!(state.browse_category, category);
            assert!(state.hit_regions.miller_columns.is_none());
            assert!(state.hit_regions.alphabet_strip.is_none());
            assert!(state.hit_regions.track_pane.is_none());
            let pane = state.hit_regions.search_landing.unwrap();
            for kind in [
                MouseEventKind::Down(MouseButton::Left),
                MouseEventKind::Down(MouseButton::Right),
                MouseEventKind::ScrollDown,
            ] {
                assert!(mouse_input::handle_mouse(
                    MouseEvent {
                        kind,
                        column: pane.x + 2,
                        row: pane.y + 2,
                        modifiers: KeyModifiers::NONE,
                    },
                    &mut state
                )
                .is_empty());
                assert_eq!(state.category_column_index, 0);
                assert_eq!(state.browse_category, category);
            }
            key_input::handle_key(
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                &mut state,
                &Config::default(),
            );
            draw(&mut state, 150, 50);
            assert!(state.hit_regions.search_landing.is_none());
        }
    }
}

#[test]
fn unfocused_and_non_theme_rows_show_the_applied_theme() {
    let mut state = AppState::new();
    state.view = View::Settings;
    state.settings_state.section = SettingsSection::Textamp;
    state.theme = *ThemeName::all().last().unwrap();
    for focus in [SettingsFocus::Sections, SettingsFocus::Content] {
        state.settings_state.focus = focus;
        state.settings_state.item_index = if focus == SettingsFocus::Content {
            state.textamp_settings().len() - 1
        } else {
            0
        };
        let terminal = draw(&mut state, 160, 40);
        let list = state.hit_regions.settings_textamp.as_ref().unwrap().inner;
        assert_eq!(
            terminal.backend().buffer()[(list.right(), list.y)].bg,
            Theme::new(state.theme).colors.bg_primary
        );
    }
}

#[test]
fn highlighted_theme_previews_without_applying_and_preview_is_not_clickable() {
    let mut state = AppState::new();
    state.view = View::Settings;
    state.settings_state.section = SettingsSection::Textamp;
    state.settings_state.focus = SettingsFocus::Content;
    let applied = state.theme;
    for name in ThemeName::all() {
        state.settings_state.item_index = state
            .textamp_settings()
            .iter()
            .position(|item| *item == TextampSetting::Theme(*name))
            .unwrap();
        let terminal = draw(&mut state, 160, 40);
        let list = state.hit_regions.settings_textamp.as_ref().unwrap().inner;
        let x = list.right();
        assert!(x < 100, "space remains to the right of the list");
        let color = terminal.backend().buffer()[(x, list.y)].bg;
        assert_eq!(color, Theme::new(*name).colors.bg_primary);
        let selected = state.settings_state.item_index;
        assert!(mouse_input::handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: x + 3,
                row: list.y + 3,
                modifiers: KeyModifiers::NONE,
            },
            &mut state
        )
        .is_empty());
        assert_eq!(state.settings_state.item_index, selected);
        assert_eq!(state.theme, applied, "rendering must not apply a theme");
    }
}

#[test]
fn themed_logo_stays_inside_its_bounds_at_small_sizes_and_keeps_about_colors() {
    use textamp::ui::widgets::logo;
    let original = logo::original(ratatui::style::Color::Black);
    for name in ThemeName::all() {
        for (width, height) in [(0, 0), (1, 1), (12, 3), (35, 10), (80, 20)] {
            let area = Rect::new(2, 2, width, height);
            let mut terminal = Terminal::new(TestBackend::new(90, 25)).unwrap();
            terminal
                .draw(|frame| logo::render_themed(frame, area, &Theme::new(*name).colors))
                .unwrap();
            let buffer = terminal.backend().buffer();
            for y in 0..25 {
                for x in 0..90 {
                    if !area.contains((x, y).into()) {
                        assert_eq!(buffer[(x, y)], ratatui::buffer::Cell::default());
                    }
                }
            }
        }
    }
    assert_eq!(logo::original(ratatui::style::Color::Black), original);
    assert!(original
        .iter()
        .flat_map(|l| &l.spans)
        .any(|s| matches!(s.style.fg, Some(ratatui::style::Color::Rgb(_, _, _)))));
}

#[test]
fn question_mark_is_not_a_shortcut_and_ctrl_f_still_opens_search() {
    use textamp::app::{action::SearchAction, Action};
    for view in [
        View::Browse,
        View::NowPlaying,
        View::Queue,
        View::Settings,
        View::Help,
    ] {
        for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
            let mut state = AppState::new();
            state.view = view;
            let config = Config::default();
            let actions = key_input::handle_key(
                KeyEvent::new(KeyCode::Char('?'), modifiers),
                &mut state,
                &config,
            );
            assert!(actions.is_empty(), "? in {view:?}: {actions:?}");
            assert_eq!(state.view, view);
            assert!(!state.popups.search_active);
            let actions = key_input::handle_key(
                KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
                &mut state,
                &config,
            );
            assert!(matches!(
                actions.as_slice(),
                [Action::Search(SearchAction::OpenSearchPopup)]
            ));
            let terminal = draw(&mut state, 160, 40);
            let bar: String = (0..160)
                .map(|x| terminal.backend().buffer()[(x, 39)].symbol())
                .collect();
            assert!(!bar.contains('?'));
        }
    }
}
