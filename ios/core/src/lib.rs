//! C ownership boundary for the native iOS presentation.
use std::ffi::{CStr, CString, c_char};
use textamp::mobile::{Client, Options};

/// Owned byte buffer. Release exactly once with `textamp_bytes_free`.
#[repr(C)]
pub struct Bytes {
    /// Pointer to `length` bytes, or null for an absent result.
    pub data: *mut u8,
    /// Byte count; no trailing terminator is required.
    pub length: usize,
}
fn bytes(value: Option<Vec<u8>>) -> Bytes {
    let Some(value) = value else {
        return Bytes {
            data: std::ptr::null_mut(),
            length: 0,
        };
    };
    let mut value = value.into_boxed_slice();
    let result = Bytes {
        data: value.as_mut_ptr(),
        length: value.len(),
    };
    std::mem::forget(value);
    result
}

/// Start a runtime. Returns null and writes an owned error string on failure.
/// # Safety
/// `options` must be a valid UTF-8 C string. `error` must point to writable pointer storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn textamp_start(
    options: *const c_char,
    error: *mut *mut c_char,
    output_session: extern "C" fn(bool) -> bool,
) -> *mut Client {
    // Safety: the caller supplies valid terminated input and writable output storage.
    let result = unsafe { CStr::from_ptr(options) }
        .to_str()
        .map_err(anyhow_string)
        .and_then(|text| serde_json::from_str::<Options>(text).map_err(anyhow_string))
        .and_then(|options| {
            Client::start(options, move |active| {
                if output_session(active) {
                    Ok(())
                } else {
                    Err(std::io::Error::other("iOS could not change the audio session").into())
                }
            })
            .map_err(anyhow_string)
        });
    match result {
        Ok(client) => Box::into_raw(Box::new(client)),
        Err(message) => {
            // Safety: `error` belongs to the caller and receives one owned allocation.
            unsafe {
                *error = CString::new(message.replace('\0', " "))
                    .map(CString::into_raw)
                    .unwrap_or(std::ptr::null_mut())
            };
            std::ptr::null_mut()
        }
    }
}
fn anyhow_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Submit normalized input. Null means success; otherwise release the returned error string.
/// # Safety
/// `client` is a live handle from `textamp_start`; `command` is a valid UTF-8 C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn textamp_send(
    client: *const Client,
    command: *const c_char,
) -> *mut c_char {
    // Safety: the native serial bridge keeps the client alive while borrowing its input.
    let result = unsafe { CStr::from_ptr(command) }
        .to_str()
        .map_err(anyhow_string)
        .and_then(|text| unsafe { &*client }.send(text).map_err(anyhow_string));
    result
        .err()
        .and_then(|message| CString::new(message.replace('\0', " ")).ok())
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}
/// Take the latest model and player updates as JSON.
/// # Safety
/// `client` must be live for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn textamp_drain(client: *const Client) -> Bytes {
    // Safety: the caller retains the handle until the serial bridge stops.
    let update = unsafe { &*client }.drain();
    bytes(serde_json::to_vec(&update).ok())
}
/// Copy cached artwork bytes into a caller-owned buffer.
/// # Safety
/// `client` is live and `id` is a valid terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn textamp_asset(client: *const Client, id: *const c_char) -> Bytes {
    // Safety: the caller retains both borrowed inputs during the call.
    let value = unsafe { CStr::from_ptr(id) }
        .to_str()
        .ok()
        .and_then(|id| unsafe { &*client }.asset(id));
    bytes(value.map(|data| data.as_ref().clone()))
}
/// Encode immutable artwork into ANSI or braille cells as JSON.
/// # Safety
/// `data` points to `length` readable bytes and `mode` is terminated UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn textamp_character_art(
    data: *const u8,
    length: usize,
    mode: *const c_char,
) -> Bytes {
    if data.is_null() || length == 0 {
        return bytes(None);
    }
    // # Safety: the caller retains the byte slice and mode for this synchronous call.
    let (data, mode) = unsafe {
        (
            std::slice::from_raw_parts(data, length),
            CStr::from_ptr(mode),
        )
    };
    let value = mode
        .to_str()
        .ok()
        .and_then(|mode| textamp::mobile::character_art(data, mode));
    bytes(value.and_then(|value| serde_json::to_vec(&value).ok()))
}
/// Shut down tasks and audio, then release the handle. Call away from the main thread.
/// # Safety
/// `client` was returned by `textamp_start`, is owned by the caller, and is never used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn textamp_stop(client: *mut Client) {
    if !client.is_null() {
        // Safety: ownership is returned exactly once by the serial native bridge.
        drop(unsafe { Box::from_raw(client) });
    }
}
/// Release a buffer returned by this library.
/// # Safety
/// `value` is an unreleased buffer returned by a Textamp function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn textamp_bytes_free(value: Bytes) {
    if !value.data.is_null() {
        // Safety: the allocation was created as a boxed slice with this length.
        drop(unsafe {
            Box::from_raw(std::ptr::slice_from_raw_parts_mut(value.data, value.length))
        });
    }
}
/// Release a returned error string.
/// # Safety
/// `value` is an unreleased string returned by `textamp_start` or `textamp_send`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn textamp_string_free(value: *mut c_char) {
    if !value.is_null() {
        // Safety: ownership of the CString allocation is returned exactly once.
        drop(unsafe { CString::from_raw(value) });
    }
}

/// Read the user-visible activity journal on the native bridge queue.
#[unsafe(no_mangle)]
pub extern "C" fn textamp_activity_history() -> Bytes {
    bytes(Some(textamp::util::activity_history_text().into_bytes()))
}

/// Record a native UI error alongside core notifications.
/// # Safety
/// The caller must supply a valid, NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn textamp_record_error(message: *const c_char) {
    // SAFETY: the Swift bridge keeps its C string alive through this call.
    if let Ok(message) = unsafe { CStr::from_ptr(message) }.to_str() {
        textamp::util::activity_error(message);
    }
}
