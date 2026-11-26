//! Native menus and their palette entries share one generated command tree.
use super::{Menu, MenuItem, NativeCommand, PaletteCommandKind};
use crate::app::{self, state::PaletteEntry};

#[derive(Clone)]
pub struct Command {
    pub entry: PaletteEntry,
    pub disabled: bool,
}

#[derive(Clone, Debug, PartialEq, gpui_kit::Action)]
#[action(no_json)]
pub struct Invoke {
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, gpui_kit::Action)]
#[action(no_json)]
pub struct Native {
    pub path: Vec<String>,
    pub label: String,
    pub action_name: String,
}

impl Native {
    pub fn available(&self, origin: Option<&Origin>) -> bool {
        self.path.first().is_none_or(|menu| menu != "Edit")
            || origin.is_some_and(|origin| origin.actions.contains(&self.action_name))
    }
    pub fn key(&self) -> String {
        format!("native:{}/{}", self.path.join("/"), self.label)
    }
}

#[derive(Clone)]
pub struct Origin {
    pub window: gpui_kit::AnyWindowHandle,
    pub focus: Option<gpui_kit::FocusHandle>,
    pub actions: Vec<String>,
}
impl gpui_kit::Global for Origin {}

pub fn catalog(state: Option<&app::AppState>) -> Vec<Command> {
    let entries = state
        .map(app::command_palette::materialize_entries)
        .unwrap_or_else(app::command_palette::global_entries);
    entries
        .into_iter()
        .map(|mut entry| {
            app::command_catalog::prepare(&mut entry);
            let disabled = state.is_some_and(|state| {
                app::command_catalog::disabled_reason(state, &entry.command).is_some()
            });
            Command { entry, disabled }
        })
        .collect()
}

pub fn resolve(key: &str, commands: &[Command]) -> Option<PaletteCommandKind> {
    commands
        .iter()
        .find(|item| app::command_history::key(&item.entry.command).as_deref() == Some(key))
        .map(|item| item.entry.command.clone())
}

fn insert(items: &mut Vec<MenuItem>, path: &[String], item: MenuItem) {
    let Some((head, tail)) = path.split_first() else {
        items.push(item);
        return;
    };
    let index = items
        .iter()
        .position(|item| matches!(item, MenuItem::Submenu(menu) if menu.name.as_ref() == head));
    let index = index.unwrap_or_else(|| {
        items.push(MenuItem::submenu(Menu {
            name: head.clone().into(),
            disabled: false,
            items: vec![],
        }));
        items.len() - 1
    });
    if let MenuItem::Submenu(menu) = &mut items[index] {
        insert(&mut menu.items, tail, item);
    }
}

pub fn menus(commands: &[Command]) -> Vec<Menu> {
    let mut menus: Vec<_> = [
        "Textamp", "File", "Edit", "View", "Playback", "Queue", "Discover", "Window", "Help",
    ]
    .into_iter()
    .map(|name| Menu {
        name: name.into(),
        disabled: false,
        items: vec![],
    })
    .collect();
    for item in commands {
        let Some(key) = app::command_history::key(&item.entry.command) else {
            continue;
        };
        let path = app::command_catalog::path(&item.entry.command);
        let Some((root, tail)) = path.split_first() else {
            continue;
        };
        let Some(menu) = menus.iter_mut().find(|menu| menu.name.as_ref() == root) else {
            continue;
        };
        let mut leaf = MenuItem::action(item.entry.label.clone(), Invoke { key });
        if let MenuItem::Action { disabled, .. } = &mut leaf {
            *disabled = item.disabled;
        }
        insert(&mut menu.items, tail, leaf);
    }
    let app = &mut menus[0].items;
    let quit = app.iter().position(|item| matches!(item, MenuItem::Action {action,..} if action.as_any().downcast_ref::<Invoke>().is_some_and(|invoke|invoke.key == "Quit"))).map(|index|app.remove(index));
    app.push(MenuItem::separator());
    app.push(MenuItem::os_submenu(
        "Services",
        gpui_kit::SystemMenuType::Services,
    ));
    app.extend([
        MenuItem::action("Hide Textamp", super::Hide),
        MenuItem::action("Hide Others", super::HideOthers),
        MenuItem::action("Show All", super::ShowAll),
    ]);
    if let Some(quit) = quit {
        app.push(MenuItem::separator());
        app.push(quit);
    }
    let edit = &mut menus[2].items;
    use gpui_kit::component::input;
    edit.extend([
        MenuItem::action("Undo", input::Undo),
        MenuItem::action("Redo", input::Redo),
        MenuItem::separator(),
        MenuItem::action("Cut", input::Cut),
        MenuItem::action("Copy", input::Copy),
        MenuItem::action("Paste", input::Paste),
        MenuItem::action("Select All", input::SelectAll),
    ]);
    let view = &mut menus[3].items;
    view.insert(0, MenuItem::action("Command Palette", super::Palette));
    view.insert(0, MenuItem::action("Reset View", super::ResetView));
    #[cfg(target_os = "macos")]
    menus[7].items.push(MenuItem::action(
        "Audio Player…",
        super::player_devices::Players,
    ));
    menus[7].items.extend([
        MenuItem::action("Minimize", super::Minimize),
        MenuItem::action("Close Window", super::CloseWindow),
        MenuItem::action("Library", super::Library),
        MenuItem::action("Now Playing", super::NowPlaying),
        MenuItem::action(
            "Play Queue",
            NativeCommand(super::NativeCommandKind::GotoQueue),
        ),
        MenuItem::action("Settings…", super::Settings),
    ]);
    menus
}

/// Platform-only commands are collected from the same leaves as the menu bar.
pub fn native_entries(menus: &[Menu]) -> Vec<super::surface::Item> {
    fn visit(menu: &Menu, path: Vec<String>, out: &mut Vec<super::surface::Item>) {
        for item in &menu.items {
            match item {
                MenuItem::Submenu(child) => {
                    let mut path = path.clone();
                    path.push(child.name.to_string());
                    visit(child, path, out);
                }
                MenuItem::Action { name, action, .. } if !action.as_any().is::<Invoke>() => {
                    let native = Native {
                        path: path.clone(),
                        label: name.to_string(),
                        action_name: action.name().into(),
                    };
                    out.push(super::surface::Item {
                        glyph: None,
                        key: native.key(),
                        label: native.label.clone(),
                        aliases: vec![format!("{} {}", path.join(" "), native.label)],
                        selected: false,
                        operation: super::surface::Operation::Native(native),
                    });
                }
                _ => {}
            }
        }
    }
    let mut out = vec![];
    for menu in menus {
        visit(menu, vec![menu.name.to_string()], &mut out);
    }
    out
}

pub fn native_action(menus: &[Menu], native: &Native) -> Option<Box<dyn gpui_kit::Action>> {
    let (root, tail) = native.path.split_first()?;
    let mut menu = menus.iter().find(|menu| menu.name.as_ref() == root)?;
    for name in tail {
        menu = menu.items.iter().find_map(|item| match item {
            MenuItem::Submenu(child) if child.name.as_ref() == name => Some(child),
            _ => None,
        })?;
    }
    menu.items.iter().find_map(|item| match item {
        MenuItem::Action { name, action, .. } if name.as_ref() == native.label => {
            Some(action.boxed_clone())
        }
        _ => None,
    })
}

pub fn signature(commands: &[Command]) -> Vec<(String, String, bool)> {
    commands
        .iter()
        .filter_map(|item| {
            app::command_history::key(&item.entry.command)
                .map(|key| (key, item.entry.label.clone(), item.disabled))
        })
        .collect()
}

/// Preserve each existing shortcut's context when attaching it to the generated menu action.
pub fn bind_shortcuts(cx: &mut gpui_kit::App) {
    let bindings: Vec<_> = cx.key_bindings().borrow().bindings().cloned().collect();
    let aliases = bindings
        .into_iter()
        .filter_map(|binding| {
            let action = binding.action();
            let command = if let Some(native) = action.as_any().downcast_ref::<NativeCommand>() {
                Some(native.command())
            } else if action.as_any().is::<super::Library>() {
                Some(PaletteCommandKind::GotoLibrary)
            } else if action.as_any().is::<super::NowPlaying>() {
                Some(PaletteCommandKind::GotoNowPlaying)
            } else if action.as_any().is::<super::Settings>() {
                Some(PaletteCommandKind::OpenSettings)
            } else if action.as_any().is::<super::Quit>() {
                Some(PaletteCommandKind::Quit)
            } else if action.as_any().is::<super::PlayPause>() {
                Some(PaletteCommandKind::PlayPause)
            } else if action.as_any().is::<super::CycleApplicationMode>() {
                Some(PaletteCommandKind::CycleApplicationMode)
            } else {
                None
            }?;
            let key = app::command_history::key(&command)?;
            let text = binding
                .keystrokes()
                .iter()
                .map(|key| key.unparse(cx.platform_info().os))
                .collect::<Vec<_>>()
                .join(" ");
            gpui_kit::KeyBinding::load(
                &text,
                Box::new(Invoke { key }),
                binding.predicate(),
                false,
                None,
                cx,
            )
            .ok()
        })
        .collect::<Vec<_>>();
    cx.bind_keys(aliases);
}
