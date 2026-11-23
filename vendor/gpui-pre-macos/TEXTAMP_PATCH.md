# GPUI macOS snapshot compatibility

Source: crates.io gpui-pre-macos 0.3.11 (GPUI Kit 0.9.0 dependency).
License: Apache-2.0, retained in LICENSE-APACHE.

Local change: replace the three Option::map_or_default calls with equivalent
map_or_else(Default::default, ...) calls. The former API is unstable on the
installed Rust 1.95 toolchain. No other behavior is changed.
