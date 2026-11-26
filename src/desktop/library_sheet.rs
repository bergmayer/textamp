//! AppKit controls for the per-library management sheet.
use super::{
    engine::{Command, CommandPort},
    surface::{Operation, Surface},
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::ffi::{CString, c_char, c_void};
use std::ptr::NonNull;

unsafe extern "C" {
    fn textamp_library_sheet_open(
        parent: *mut c_void,
        context: *mut c_void,
        selected: extern "C" fn(*mut c_void, isize),
    ) -> *mut c_void;
    fn textamp_library_sheet_update(sheet: *mut c_void, json: *const c_char);
    fn textamp_library_sheet_close(sheet: *mut c_void);
}

struct Actions {
    commands: CommandPort,
    id: String,
    revision: u64,
    items: Vec<Operation>,
}
extern "C" fn select(context: *mut c_void, index: isize) {
    // The main-thread sheet owns this callback context until it closes.
    let actions = unsafe { &*context.cast::<Actions>() };
    let command = if index < 0 {
        Command::CloseSurface(actions.id.clone())
    } else if let Some(op) = actions.items.get(index as usize) {
        Command::Surface {
            id: actions.id.clone(),
            revision: actions.revision,
            operation: op.clone(),
        }
    } else {
        return;
    };
    let _ = actions.commands.try_send(command);
}

pub(super) struct Sheet {
    handle: NonNull<c_void>,
    actions: Box<Actions>,
}
impl Sheet {
    pub fn open(window: &gpui_kit::Window, model: &Surface, commands: CommandPort) -> Option<Self> {
        let RawWindowHandle::AppKit(parent) = HasWindowHandle::window_handle(window).ok()?.as_raw()
        else {
            return None;
        };
        let mut actions = Box::new(Actions {
            commands,
            id: model.id.clone(),
            revision: model.revision,
            items: vec![],
        });
        let handle = NonNull::new(unsafe {
            textamp_library_sheet_open(
                parent.ns_view.as_ptr(),
                (&mut *actions as *mut Actions).cast(),
                select,
            )
        })?;
        let mut sheet = Self { handle, actions };
        sheet.update(model);
        Some(sheet)
    }
    pub fn update(&mut self, model: &Surface) {
        if self.actions.revision == model.revision && !self.actions.items.is_empty() {
            return;
        }
        let tabs: Vec<_> = model.tabs.iter().map(|i| &i.label).collect();
        let controls: Vec<_> = model
            .toggles
            .iter()
            .map(|i| serde_json::json!({"label":i.label,"checked":i.checked}))
            .chain(
                model
                    .buttons
                    .iter()
                    .map(|i| serde_json::json!({"label":i.label})),
            )
            .collect();
        let payload = serde_json::json!({"title":model.title,"tabs":tabs,"page":model.tabs.iter().position(|i|i.selected),"details":model.text.lines().collect::<Vec<_>>(),"controls":controls});
        self.actions.items = model
            .tabs
            .iter()
            .map(|i| i.operation.clone())
            .chain(model.toggles.iter().map(|i| i.operation.clone()))
            .chain(model.buttons.iter().map(|i| i.operation.clone()))
            .collect();
        self.actions.revision = model.revision;
        let json = CString::new(payload.to_string()).expect("JSON escapes NUL");
        unsafe { textamp_library_sheet_update(self.handle.as_ptr(), json.as_ptr()) };
    }
}
impl Drop for Sheet {
    fn drop(&mut self) {
        unsafe { textamp_library_sheet_close(self.handle.as_ptr()) };
    }
}
