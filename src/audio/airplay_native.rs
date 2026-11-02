//! Source-built AirPlay sender, invoked only in a child of this executable.
//! Its C entry point owns process-wide signals, stdio and exit handling.
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;

pub const ARGUMENT: &str = "--internal-airplay";

unsafe extern "C" {
    fn textamp_airplay_main(argc: libc::c_int, argv: *mut *mut libc::c_char) -> libc::c_int;
}

/// Called before app initialization, the process lock or audio-device access.
/// Keeping the native runtime in its own process also preserves cancellation.
pub fn run(arguments: impl Iterator<Item = std::ffi::OsString>) -> i32 {
    // Reference the crate so Cargo links the statically built OpenSSL runtime.
    openssl_sys::init();
    let mut arguments: Vec<_> = std::iter::once(std::ffi::OsString::from("textamp-airplay"))
        .chain(arguments)
        .map(|argument| CString::new(argument.as_os_str().as_bytes()))
        .collect::<Result<_, _>>()
        .expect("process arguments cannot contain NUL");
    let mut pointers: Vec<_> = arguments
        .iter_mut()
        .map(|argument| argument.as_ptr().cast_mut())
        .collect();
    let count = pointers.len() as libc::c_int;
    pointers.push(std::ptr::null_mut());
    // SAFETY: argv is NUL terminated and every CString outlives the call.
    // This entry point is invoked once in a dedicated subprocess, never in the UI.
    unsafe { textamp_airplay_main(count, pointers.as_mut_ptr()) }
}

pub fn notices() -> String {
    format!(
        "Textamp includes Music Assistant airplay-cli v0.5.8 and its source dependencies.\nThe combined executable is distributed under GPL-3.0.\nTextamp's original Rust source remains under the Unlicense.\nOpenSSL 3 is built from Cargo source dependencies (replacing upstream OpenSSL 1.1.1).\nSource: https://github.com/bergmayer/textamp/tree/master/vendor/airplay\n\n{}\n\n{}\n\n{}",
        include_str!("../../vendor/airplay/LICENSE"),
        include_str!("../../vendor/airplay/THIRD_PARTY_NOTICES.md"),
        include_str!("../../vendor/airplay/OPENSSL-LICENSE.txt"),
    )
}
