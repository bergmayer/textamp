//! Main-thread AppKit window suspension without destroying frontend entities or drafts.
use gpui_kit::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
unsafe extern "C" {
    fn textamp_register_library_view(view: *mut std::ffi::c_void);
    fn textamp_platform_modal_active() -> u32;
    fn textamp_suspend_auxiliary_windows(suspend: u32);
}
pub(super) fn register(window: &Window) {
    if let Ok(handle) = HasWindowHandle::window_handle(window)
        && let RawWindowHandle::AppKit(handle) = handle.as_raw()
    {
        // # Safety: GPUI owns this live NSView, and this adapter runs on the app thread.
        unsafe { textamp_register_library_view(handle.ns_view.as_ptr()) };
    }
}
pub(super) fn modal_active() -> bool {
    // # Safety: called on GPUI's main thread; the function only inspects AppKit objects.
    unsafe { textamp_platform_modal_active() != 0 }
}
pub(super) fn suspend(suspend: bool) {
    // # Safety: called on GPUI's main thread; AppKit retains ownership of every window.
    unsafe { textamp_suspend_auxiliary_windows(u32::from(suspend)) };
}
