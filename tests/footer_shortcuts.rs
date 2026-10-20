use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, Terminal};
use textamp::app::{
    action::SystemAction,
    handlers::{dispatch_system, key_input, mouse_input},
    state::{SettingsFocus, View},
    Action, AppState,
};
use textamp::config::Config;

fn state(view: View, width: u16, tall: bool) -> AppState {
    let mut app = AppState::new();
    app.view = view;
    app.tall_mode = tall;
    app.terminal_width = width;
    app.terminal_height = 32;
    app.settings_state.focus = SettingsFocus::Content;
    let mut terminal = Terminal::new(TestBackend::new(width, 32)).unwrap();
    terminal
        .draw(|frame| app.apply_render_feedback(textamp::ui::render(frame, &app)))
        .unwrap();
    app
}

fn click(app: &mut AppState, key: KeyCode) -> Vec<Action> {
    let rect = app
        .hit_regions
        .footer_shortcuts
        .iter()
        .find(|(_, k)| *k == key)
        .unwrap()
        .0;
    mouse_input::handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        },
        app,
    )
}

#[tokio::test]
async fn footer_clicks_use_keyboard_actions_without_adding_focus_stops() {
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    for (view, tall, keys) in [
        (
            View::Browse,
            false,
            vec![
                KeyCode::F(7),
                KeyCode::F(8),
                KeyCode::Char(':'),
                KeyCode::Char('/'),
                KeyCode::Tab,
                KeyCode::Char(','),
                KeyCode::Char('\\'),
                KeyCode::Char('|'),
            ],
        ),
        (
            View::Queue,
            true,
            vec![
                KeyCode::F(7),
                KeyCode::F(8),
                KeyCode::Char(':'),
                KeyCode::Char('/'),
                KeyCode::Tab,
                KeyCode::Char(','),
                KeyCode::Char('|'),
            ],
        ),
        (View::Settings, false, vec![KeyCode::F(2), KeyCode::F(10)]),
    ] {
        for key in keys {
            let mut mouse = state(view, 140, tall);
            let mut keyboard = state(view, 140, tall);
            let expected = key_input::handle_key(
                KeyEvent::new(key, KeyModifiers::NONE),
                &mut keyboard,
                &Config::default(),
            );
            let actions = click(&mut mouse, key);
            let [Action::System(SystemAction::ActivateFooterShortcut(code))] = actions.as_slice()
            else {
                panic!("{actions:?}")
            };
            assert_eq!(*code, key);
            let actual = dispatch_system::dispatch(
                &tx,
                &mut Config::default(),
                SystemAction::ActivateFooterShortcut(*code),
                &mut mouse,
            )
            .await
            .unwrap();
            assert_eq!(
                format!("{actual:?}"),
                format!("{expected:?}"),
                "{view:?}: {key:?}"
            );
            assert_eq!(mouse.palette.open, keyboard.palette.open);
            assert_eq!(mouse.settings_state.focus, keyboard.settings_state.focus);
            assert_eq!(
                mouse.category_column_focused,
                keyboard.category_column_focused
            );
            assert_eq!(mouse.view, keyboard.view);
        }
    }
}

#[tokio::test]
async fn footer_controls_end_filter_entry_and_modals_block_click_through() {
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let mut app = state(View::Browse, 140, false);
    app.list_filter.active = true;
    app.list_filter.query = "test".into();
    let actions = click(&mut app, KeyCode::Char(':'));
    let [Action::System(action)] = actions.as_slice() else {
        panic!("{actions:?}")
    };
    dispatch_system::dispatch(&tx, &mut Config::default(), action.clone(), &mut app)
        .await
        .unwrap();
    assert!(app.palette.open);
    assert!(!app.list_filter.active);
    assert!(
        click(&mut app, KeyCode::F(7)).is_empty(),
        "Palette consumes outside click; no full screen activation"
    );
    app.notifications.last_error = Some("fixture error".into());
    assert!(click(&mut app, KeyCode::Char(',')).is_empty());
}

#[test]
fn footer_hit_targets_are_clipped_to_visible_labels_and_exclude_separators() {
    for width in [28, 40, 60, 80, 100, 140] {
        let mut app = state(View::Browse, width, false);
        let mut previous_end = 0;
        for (rect, _) in &app.hit_regions.footer_shortcuts {
            assert!(rect.width > 0);
            assert!(rect.x >= previous_end);
            assert!(rect.right() <= width);
            assert_eq!(rect.y, 31);
            previous_end = rect.right();
        }
        if width == 140 {
            assert_eq!(app.hit_regions.footer_shortcuts.len(), 8);
            let first = app.hit_regions.footer_shortcuts[0].0;
            let separator = MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: first.right(),
                row: first.y,
                modifiers: KeyModifiers::NONE,
            };
            assert!(mouse_input::handle_mouse(separator, &mut app).is_empty());
        }
    }
}
