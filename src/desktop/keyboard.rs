//! Native keyboard traversal and adapters for application commands.
use super::*;

gpui_kit::actions!(keyboard, [NextFocus, PreviousFocus]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", NextFocus, None),
        KeyBinding::new("shift-tab", PreviousFocus, None),
    ]);
    bind_navigation_shortcuts(cx);
}

pub(super) fn next_focus(_: &NextFocus, window: &mut Window, cx: &mut App) {
    window.focus_next(cx);
    cx.stop_propagation();
}

pub(super) fn previous_focus(_: &PreviousFocus, window: &mut Window, cx: &mut App) {
    window.focus_prev(cx);
    cx.stop_propagation();
}

pub(super) fn bind_navigation_shortcuts(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "f7",
        NativeCommand(NativeCommandKind::ToggleFullscreen),
        None,
    )]);
    for binding in ["cmd-alt-t", "ctrl-alt-t", "cmd-shift-t", "ctrl-shift-t"] {
        cx.bind_keys([KeyBinding::new(binding, CycleApplicationMode, None)]);
    }
    for shortcut in crate::app::shortcuts::NAVIGATION {
        for binding in shortcut.windowed {
            use crate::app::shortcuts::NavigationCommand;
            let binding = match shortcut.command {
                NavigationCommand::Radio
                | NavigationCommand::Journey
                | NavigationCommand::AlbumMix
                | NavigationCommand::LibraryMix => {
                    let key = crate::app::command_history::key(&shortcut.command.command())
                        .expect("global navigation command");
                    KeyBinding::new(binding, menu_catalog::Invoke { key }, None)
                }
                NavigationCommand::Library => KeyBinding::new(binding, Library, None),
                NavigationCommand::NowPlaying => KeyBinding::new(binding, NowPlaying, None),
                NavigationCommand::Settings => KeyBinding::new(binding, Settings, None),
                NavigationCommand::Queue => {
                    KeyBinding::new(binding, NativeCommand(NativeCommandKind::GotoQueue), None)
                }
                NavigationCommand::SearchAppleMusic => KeyBinding::new(
                    binding,
                    menu_catalog::Invoke {
                        key: "SearchAppleMusic".into(),
                    },
                    None,
                ),
                NavigationCommand::RandomAlbum => KeyBinding::new(
                    binding,
                    NativeCommand(NativeCommandKind::GoToRandomAlbum),
                    None,
                ),
            };
            cx.bind_keys([binding]);
        }
    }
    for binding in ["cmd-shift-p", "ctrl-shift-p", "cmd-p"] {
        cx.bind_keys([KeyBinding::new(binding, Palette, None)]);
    }
}
