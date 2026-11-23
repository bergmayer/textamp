# Terminal Note icons

The music mark and command prompt use a fixed character-cell grid. The shared
renderer in `src/icon.rs` derives each variant from the canonical theme palette,
with contrast correction for light and monochrome appearances. System includes
light and dark variants.

Run `cargo run --example app_icons` to regenerate the iOS app-icon catalogs,
macOS ICNS resource and named PNG exports here. iOS supplies its own icon mask;
macOS exports include a transparent inset around the rounded plate.

`terminal-note-reference.png` is the visual master for the cell-based design.
