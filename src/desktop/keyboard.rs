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

#[derive(Clone, Copy)]
pub(super) enum Pane {
    Sidebar,
    Library,
    Queue,
    Rail,
    Transport,
}

pub(super) struct PaneFocus([FocusHandle; 5]);

impl PaneFocus {
    pub fn new(cx: &mut App) -> Self {
        Self(std::array::from_fn(|index| {
            cx.focus_handle()
                .tab_stop(index == Pane::Queue as usize || index == Pane::Library as usize)
        }))
    }

    pub fn handle(&self, pane: Pane) -> &FocusHandle {
        &self.0[pane as usize]
    }

    pub fn step(
        &self,
        backwards: bool,
        within_pane: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        step_pane_focus(&self.0, backwards, within_pane, window, cx)
    }
}

/// Walk the native focus order, retaining only stops in the requested pane scope.
pub(super) fn step_pane_focus(
    panes: &[FocusHandle],
    backwards: bool,
    within_pane: bool,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let current = |window: &Window, cx: &App| {
        panes
            .iter()
            .position(|pane| pane.contains_focused(window, cx))
    };
    let initial = window.focused(cx);
    let pane = current(window, cx);
    if within_pane && pane.is_none() {
        return false;
    }
    let mut first = None;
    loop {
        if backwards {
            window.focus_prev(cx);
        } else {
            window.focus_next(cx);
        }
        let focused = window.focused(cx);
        if focused.is_none() || focused == initial || focused == first {
            if let Some(initial) = initial {
                initial.focus(window, cx);
            }
            break;
        }
        if (current(window, cx) == pane) == within_pane {
            break;
        }
        if first.is_none() {
            first = focused;
        }
    }
    cx.stop_propagation();
    true
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
