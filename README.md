# textamp

A keyboard-driven Plex Music client — available as both a terminal
(TUI) and a desktop (GUI) app from the same shared codebase. Both
front-ends cover the same features with the same keyboard shortcuts;
you pick whichever you prefer.

- **`textamp`** — terminal client (ratatui + crossterm).
  See **[README-TUI.md](README-TUI.md)**.
- **`textamp-gui`** — desktop client (Iced, with native OS menu bars
  via muda on macOS, Windows, and Linux-with-GTK3).
  See **[README-GUI.md](README-GUI.md)**.

Only one can run at a time against the shared on-disk caches; the
second instance exits immediately with a clear message.

## Building

The top-level entry points are `build.sh` (Linux/macOS/WSL) and
`build.bat` (Windows). With no flags they build BOTH binaries for the
current platform; pass a flag to build just one:

```sh
./build.sh                # both TUI + GUI (release)
./build.sh --tui          # TUI only
./build.sh --gui          # GUI only
./build.sh --windows      # cross-build the Windows GUI from WSL
                          #   (rsyncs into /mnt/c staging tree, runs MSVC)

# On Windows
build.bat                 # both TUI + GUI
build.bat --tui           # TUI only
build.bat --gui           # GUI only
```

For more control (custom features, install packages, etc.) the older
per-binary scripts still exist:

```sh
./build-tui.sh                       # TUI binary, expanded options
./build-gui.sh                       # GUI binary, expanded options
./build-gui.sh --no-native-menus     # GUI without muda (Linux without GTK3)
./build-tui.sh --makepackage         # Platform-native install package (TUI)
./build-gui.sh --makepackage         # Platform-native install package (GUI)
```

Or call Cargo directly:

```sh
cargo build --release --bin textamp                                  # TUI
cargo build --release --features "gui,native-menus" --bin textamp-gui  # GUI
```

See the per-front-end READMEs for everything else: feature tour,
keyboard shortcuts, caching behaviour, genre sources, folder
discovery, and runtime prerequisites.

## License

GPL-3.0. See `LICENSE`.
