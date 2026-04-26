# textamp (GUI)

A keyboard-driven Plex Music client for the desktop. Pairs with the
terminal app (`textamp` — see [README-TUI.md](README-TUI.md)) — they
share the same shared back-end, on-disk caches, and feature set, so
you can switch between them freely.

The GUI is built on [iced](https://iced.rs/) with native OS menu bars
via [muda](https://github.com/tauri-apps/muda) (macOS, Windows, and
Linux-with-GTK3). Every TUI shortcut works here too; the GUI also
adds mouse-driven affordances (right-click context menus,
drag-to-reorder in the queue, click-already-selected = activate, etc.).

## Quick start

After signing in to Plex, the GUI opens on the **Library** view: a
left **category column** (Library / Playlists / Genres / Folders),
followed by **Miller columns** that drill into whatever you click.
The bottom bar hosts the **transport** (prev / play-pause / next, seek
slider) and a tab strip that switches between **Library** and **Now
Playing**. To the right of the tab strip is a **filter / search**
input that's always visible:

- Type to live-filter the focused list.
- Press **Enter** to leave fast-filter mode and open the global
  Search popup pre-seeded with your term.
- The little `x` clears the filter.

Press **`F2`** for Settings (Account / Textamp / Cache tabs) and
**`F1`** for the modal **Keyboard Shortcuts** popup.

## Now Playing

The Now Playing view has the album artwork, current queue / radio
list, and a stack of toggle buttons in the left sidebar. Each button
sticks in its **pressed** state when the related panel or feature is
active:

- **Radio** — opens the Stations picker. Stays pressed while a
  station is playing.
- **Play Random Album** — kicks off Plex's `randomAlbum` station.
- **Visualizer** — toggles the waveform / spectrum / spectrogram
  panel.
- **DJ Modes** — opens the DJ Modes picker (modal). Stays pressed
  while any DJ mode is active.
- **Remix Tools** — opens the Remix Tools picker (Remix: Gemini /
  Twofer / Stretch / Doppelganger / Shuffle, plus Clear Queue and
  Save as Playlist). One-shot actions; the popup closes after each
  click.

## Genres

The Genres view (`Ctrl+G`) provides six tabs reflecting different
genre sources. Some are written by you (file tags), others come from
Plex's own analysis or normalization:

- **All**: Merged list of every genre below, with a type suffix for
  disambiguation (e.g., "Rock (Library)", "Rock (Artist)").
- **Library**: Genre tags **from your music files** (e.g., "Abstract
  Improvisation", "Post-Punk"). These are the strings you've written
  into your audio metadata; Plex passes them through verbatim.
- **Artist** / **Album**: **Plex-generated** genre categories,
  derived by Plex from its own catalogue mapping and shown at the
  artist or album level (e.g., "Rock", "Jazz"). These are normalised
  into a smaller, broader vocabulary than the Library tags.
- **Mood**: **Plex analysis-based** mood metadata (e.g., "Energetic",
  "Melancholic"). Plex computes these server-side; only available
  when Plex has analysed your library.
- **Style**: **Plex analysis-based** style metadata (e.g.,
  "Avant-Garde", "Ambient"). Same source as Mood.

## Folders aren't part of the Plex API

Plex does not expose your library's folder hierarchy through its API
the way it exposes Artists / Albums / Tracks / Playlists. So textamp
discovers folders on its own:

- **Top-level folders** are scanned automatically when you first open
  the Folders view.
- **Subfolders** are cached lazily as you drill into them — without
  this, every folder navigation would be a fresh round-trip to the
  server.
- **Manual deep crawl**: open `Settings (F2) → Cache` and click
  **Start subfolder crawl** for libraries where you want fast,
  always-cached browsing of the full tree. Useful for bootlegs,
  spoken-word libraries, anything better navigated as files than
  albums.
- At 32+ days a stale subfolder entry is still served from cache as a
  "warm cache" and re-fetched in background on access.
- `F5` refreshes any folder that seems outdated.

## Caching

Library data (artists, albums, playlists, genres, stations) is cached
to disk per-library. On startup, cached data loads immediately so you
can browse without waiting for your Plex server. Fresh data is fetched
in the background and merged automatically.

- **72-hour refresh** triggers a background refresh when you navigate
  to that view.
- **32-day warm cache**: stale entries are served immediately and
  re-fetched in background on access.
- `F5` forces a refresh of the current view.
- Each library has its own cache, preserved when switching.

`Settings (F2) → Cache` shows per-category counts and ages, plus
buttons to clear the **library cache**, **artwork cache**, and
**subfolder cache** independently.

## Themes

`Settings → Textamp → Theme`:

- **Solarized Dark** (default), **Solarized Light**
- **Dark** (Plexamp-orange accent)
- **Borland** (Norton Commander blue / cyan / yellow)
- **Platinum** (Mac OS 9 grey + highlight blue)
- **Black and White** (pure black + pure white only)

Theme choice persists in `config.toml` and is shared with the TUI.

## Help

`Help → User Guide` opens the README in a modal popup; `Help →
Keyboard Shortcuts` opens the same shortcut reference the TUI shows
under `F1`. Both are dismissed with **Esc** or the **Close** button.

## Building

The top-level entry point is `build.sh` (Linux/macOS/WSL) or
`build.bat` (Windows):

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

Or call Cargo directly:

```sh
cargo build --release --features "gui,native-menus" --bin textamp-gui
```

For more control (custom features, install packages, etc.) the older
per-binary `build-gui.sh` script remains.

### Runtime prerequisites

- **macOS / Windows**: no system dependencies; muda's native menu
  integration is built-in.
- **Linux** with native menus: GTK3 must be installed
  (`sudo pacman -S gtk3`, `sudo apt install libgtk-3-dev`,
  `sudo dnf install gtk3-devel`). Without it, build with
  `--no-native-menus` (`build-gui.sh --no-native-menus`) for an
  in-window menu fallback.

Only one instance of textamp — TUI or GUI — can run at a time
(an advisory lock at `<state_dir>/textamp.lock` prevents both from
fighting over the shared caches). Quit one before launching the other.

## File Locations

textamp checks XDG environment variables first, then falls back to
platform defaults. Both binaries read and write the same `config.toml`,
`auth.toml`, library cache, waveform cache, artwork cache, and
spectrogram cache.

## License

GPL-3.0. See `LICENSE`.
