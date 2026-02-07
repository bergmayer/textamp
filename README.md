# textamp

A keyboard-driven Terminal User Interface (TUI) client for Plex Music, written in Rust.

textamp is a specialized, lightweight alternative to Plexamp for power users who live in the terminal. Inspired by musikcube's efficient navigation.

## Features

- **Keyboard-driven**: CUA-style navigation with Ctrl+key and Alt+key shortcuts
- **Hierarchical browsing**: Artist → Albums → Tracks navigation
- **Library views**: Artists, Playlists, Genres, Folders, and Stations
- **Sonic similarity**: Discover similar albums (context-aware)
- **Plexamp Stations**: Library Radio, Deep Cuts, Time Travel, Mood, Decade Radio
- **Sonic Radio**: Generate radio from any track, album, or artist using sonic similarity
- **Sonic Adventure**: Create a sonic bridge between two tracks
- **Album artwork**: Displays cover art in supported terminals (Kitty, iTerm2, Sixel)
- **Search & Filter**: Global search and tabbed filtering (Artists, Album Artists, Albums, Playlists, Tracks, Genres)
- **Folder browsing**: Miller columns view (like macOS Finder)
- **Settings screen**: Account, libraries, playback, and themes
- **Fast startup**: Library data cached to disk for instant display, refreshes in background
- **High-quality playback**: Direct streaming without transcoding

## Speed

textamp is designed to feel instant. Every interaction — browsing, searching, switching libraries, jumping to a letter — should happen without perceptible delay.

### Caching

Library data (artists, albums, playlists, genres, stations) is cached to disk per-library. On startup, cached data loads immediately so you can browse without waiting for your Plex server. Fresh data is fetched in the background and merged automatically — a toast notification appears if anything changed.

- **72-hour refresh**: Cache older than 72 hours triggers a background refresh
- **32-day refresh**: Very stale data is automatically refreshed when idle (2+ minutes)
- **Manual refresh**: `F5` forces a refresh of the current view
- **Per-library**: Each library has its own cache, preserved when switching
- **Auto-save**: Cache saves periodically while idle and on quit
- **Track not found**: If playback fails with a 404, you'll be prompted to refresh

Subfolder caches (Folders view) work differently: they're loaded lazily when you navigate into them, and very stale entries (32+ days) are deleted rather than refreshed. Press `F5` to refresh any folder that seems outdated.

To clear all cached data: Settings (F2) > Account > Clear Cache & Reload.

### Miller Columns

Artists, Genres, Playlists, Folders, and Stations all use Miller columns — the three-pane column view pioneered by macOS Finder. Selecting an item in the left column shows its children in the next column to the right. This lets you drill through Artist → Albums → Tracks (or Genre → Albums → Tracks, etc.) without loading new screens, keeping your place in each column as you navigate.

### Keyboard Navigation

Every view is navigable without a mouse. `Tab`/`Shift+Tab` moves between categories. `Shift+↓`/`Shift+↑` cycles modes within a category (e.g., Artists → Album Artists → Albums). Arrow keys, `Enter`, and `Backspace` navigate the Miller columns. `Page Up`/`Page Down`, `Home`/`End` work everywhere.

### Alphabetic Jump

Press any letter `A-Z` to jump to the first item starting with that letter. Press `Shift+[letter]` to refine within the current first letter — if you're on an item starting with "A" and press `Shift+N`, you jump to the first "An..." item (like "Andrew").

### Inline Filter

Press `/` to activate a real-time filter on the current column. Type to narrow results instantly. The filter stays active as you drill down, so you can filter artists, select one, then browse their albums without losing the filter.

### Search Popup

`Ctrl+F` opens a floating search dialog with tabs for Artists, Album Artists, Albums, Playlists, Tracks, and Genres. Selecting a result plays it without closing search, so you can keep searching.

### Radio Shortcuts

`Ctrl+Alt+L` starts Library Radio instantly. `Ctrl+Alt+R` starts Random Album Radio. `Alt+R` on any selection creates a sonic radio — sonic track radio for similar tracks, sonic album radio for similar albums, sonic artist radio for an artist's catalog. No menus, no confirmation dialogs.

### Library Switching

`Ctrl+Alt+S` opens a quick picker to switch between Plex libraries. The switch is instant — cached data for the new library loads immediately while a background refresh runs.

## Installation

### From Source

```bash
git clone https://github.com/bergmayer/textamp
cd textamp
cargo build --release
```

The binary will be at `target/release/textamp`.

### Dependencies

On Linux, you need ALSA development libraries:

```bash
# Debian/Ubuntu
sudo apt install libasound2-dev

# Fedora
sudo dnf install alsa-lib-devel

# Arch
sudo pacman -S alsa-lib
```

## File Locations

textamp checks for XDG environment variables first, then falls back to platform defaults. This allows power users to override locations if desired.

### Configuration & Data

| File | XDG Override | Linux Default | macOS Default |
|------|--------------|---------------|---------------|
| Config | `$XDG_CONFIG_HOME/textamp/config.yaml` | `~/.config/textamp/config.yaml` | `~/Library/Application Support/textamp/config.yaml` |
| Auth token | `$XDG_DATA_HOME/textamp/auth.yaml` | `~/.local/share/textamp/auth.yaml` | `~/Library/Application Support/textamp/auth.yaml` |
| Log | `$XDG_STATE_HOME/textamp/textamp.log` | `~/.local/state/textamp/textamp.log` | `~/Library/Application Support/textamp/textamp.log` |

### Cache

| File | XDG Override | Linux Default | macOS Default |
|------|--------------|---------------|---------------|
| Library cache | `$XDG_CACHE_HOME/textamp/` | `~/.cache/textamp/` | `~/Library/Caches/textamp/` |
| Waveforms | `$XDG_CACHE_HOME/textamp/waveforms/` | `~/.cache/textamp/waveforms/` | `~/Library/Caches/textamp/waveforms/` |

The library cache stores artist, album, playlist, and genre data for fast startup (refreshes in background). Waveform data is cached for the audio visualizer.

## Configuration

Configuration is optional. On first run, textamp will prompt you to sign in with your Plex account. Your auth token and selected server are stored in `auth.yaml` (not the config file).

Example `config.yaml`:

```yaml
general:
  log_level: "info"

playback:
  default_volume: 0.8
  gapless: true
  buffer_size_kb: 1024

ui:
  theme: "dark"  # Options: dark, solarized-dark, solarized-light, borland
  show_album_art: true
  album_art_size: 40

libraries:
  default_library: "1"  # Library key to open on startup
```

Advanced users can also set `plex.server_url` and `plex.token` in config to override the stored auth, but this is not recommended for normal use.

### Themes

textamp includes four built-in themes:

| Theme | Description |
|-------|-------------|
| `dark` | Default Plexamp-inspired dark theme |
| `solarized-dark` | Solarized Dark with blue accents |
| `solarized-light` | Solarized Light with magenta accents |
| `borland` | Classic Borland/Turbo Pascal style (blue background, cyan selection) |

Change themes in Settings (F2) > Interface, or set in config file.

## Keyboard Shortcuts

### Navigation
| Key | Action |
|-----|--------|
| `↑` / `↓` | Navigate lists |
| `Tab` | Next category (Artists→Playlists→Genres→Folders→Now Playing) |
| `Shift+Tab` | Previous category |
| `Shift+↓` | Cycle modes within category (e.g., Artists → Album Artists → Albums) |
| `Shift+↑` | Cycle modes backwards |
| `Enter` / `→` | Select / Drill down / Play |
| `←` / `Backspace` | Go back / Focus left |
| `Page Up/Down` | Scroll by page |
| `Home` / `End` | Jump to top/bottom |
| `A-Z` | Jump to first item starting with letter |

### Categories (Ctrl+key)
| Key | Action |
|-----|--------|
| `Ctrl+A` | Artists (cycles: Artists → Album Artists → Albums) |
| `Ctrl+P` | Playlists (cycles: All → Stations → Recently Added → Recently Played) |
| `Ctrl+G` | Genres (cycles: Genres → Artist Genres → Album Genres → Moods → Styles → Stations) |
| `Ctrl+O` | Folders |

### Views
| Key | Action |
|-----|--------|
| `Ctrl+N` | Now Playing (cycles: Queue → Now Playing) |
| `F1` / `?` | Help screen |
| `F2` | Settings |

### Commands

| Key | Action |
|-----|--------|
| `Ctrl+F` | Search popup (floating dialog) |
| `Ctrl+S` | Save queue as playlist (in Now Playing) |
| `Alt+R` | Sonic radio from selection |
| `Alt+Q` | Add selection to queue (track or album) |
| `Alt+S` | Shuffle view / queue |
| `Alt+M` | Similar albums/tracks |
| `Alt+A` | Sonic Adventure (see below) |
| `Ctrl+Alt+L` | Library Radio (station based on your library) |
| `Ctrl+Alt+R` | Random Album Radio (shuffled albums) |
| `Ctrl+Alt+S` | Quick library switcher |

### Search / Filter (Ctrl+F)

The unified search screen has tabs for different content types: Artists, Album Artists, Albums, Playlists, Tracks, and Genres. Use `Tab`/`Shift+Tab` to switch tabs. Selecting a track or album plays it while staying in search.

### Navigation Flow
- **Artists** (`Ctrl+A`): Press again to cycle between Artists, Album Artists, and Albums views
- **Playlists** (`Ctrl+P`): Press again to cycle between All Playlists, Stations, Recently Added, and Recently Played albums
- **Genres** (`Ctrl+G`): Press again to cycle between Genres, Plex Genres, and Moods
- **Folders** (`Ctrl+O`): Miller columns navigation (3 columns visible)

Drill-down paths:
- Artists/Album Artists: Artist → Albums → Tracks
- Albums: Album → Tracks
- Genres/Moods: Genre → Albums → Tracks
- Playlists: Playlist → Tracks

### Genres and Moods
The Genres view (`Ctrl+G`) provides three content types that you cycle through by pressing `Ctrl+G` again:
- **Genres**: Actual genre tags from your music files (e.g., "Abstract Improvisation", "Post-Punk")
- **Plex Genres**: Plex's standardized genre categories (e.g., "Rock", "Jazz", "Classical")
- **Moods**: Plexamp-style mood tags (e.g., "Energetic", "Melancholic")

Select a genre/mood to see albums, then drill into tracks. The view remembers your mode when you navigate away and back.

### Queue vs Radio

textamp distinguishes between two playback modes:

**Queue** (`Ctrl+N`) - A finite, user-controlled playlist:
- Play an album or playlist to populate the queue
- Queue stops at the end
- Navigate and select tracks without disrupting the queue
- Maximum 500 tracks in the queue
- ~20 tracks of play history visible above current tracks

**Sonic Radio** (`Alt+R`) - Create radio from selection using sonic similarity:
- **Sonic Track Radio**: When a track is selected, creates a radio of sonically similar individual tracks (shuffled to avoid album clustering)
- **Sonic Album Radio**: When an album is selected, plays similar albums in order (full albums sequentially)
- **Sonic Artist Radio**: When an artist is selected, plays that artist's tracks

**Stations** (via `Ctrl+G` or `Ctrl+P` cycling) - Curated Plex stations:
- Seven station types: Library, Deep Cuts, Time Travel, Random Album, Mood, Style, Decade
- Category stations (Mood, Style, Decade) drill into sub-stations via Miller columns
- Automatically fetches more tracks as needed

Only one mode is active at a time. Adding items to the queue (Alt+Q) while radio is playing converts radio to queue mode.

### Sonic Adventure

Sonic Adventure creates a "sonic bridge" between two tracks - a playlist that transitions smoothly from your start track to your end track using Plex's sonic similarity analysis.

1. Select a start track and press `Alt+A`
2. Navigate to your destination track and press `Alt+A` again
3. Enter the desired length (5-100 tracks)
4. The adventure replaces your queue and starts playing

Tracks can be selected from Browse view or from Search/Filter (Ctrl+F, Tracks tab).

### Similar Albums
The similar albums feature is context-aware:
- When viewing an artist's albums: shows similar to selected album
- When viewing tracks: shows similar to the album containing those tracks
- Otherwise: shows similar to the currently playing track's album

### Stations (Plexamp Radio)

Access stations by cycling `Ctrl+G` (Genres → ... → Stations) or `Ctrl+P` (Playlists → Stations → ...). Stations use Miller columns — the first four are directly playable, while the last three are categories you drill into to pick a sub-station.

**Directly playable:**

- **Library Radio** — Random tracks from your entire library. Each batch is a fresh random selection with no weighting.
- **Deep Cuts Radio** — Tracks you haven't played much. Sorted by play count ascending, then randomized, so rarely-heard tracks surface first.
- **Time Travel Radio** — A chronological walk through your library starting from its earliest decade. Picks a couple of albums per decade, takes a few tracks from each, then advances forward in time. Wraps around to the beginning when it reaches the end, so it plays indefinitely.
- **Random Album Radio** — Picks a random album and plays it front to back. When it finishes, fetches another random album.

**Category stations (drill in to select):**

- **Mood Radio** — Browse moods like Aggressive, Atmospheric, Energetic, Melancholic, etc. Select a mood to hear tracks tagged with it. Mood metadata lives on individual tracks, so filtering is direct.
- **Style Radio** — Browse musical styles like Rock, Jazz, Electronic, etc. Select a style to hear albums in that style. Style metadata is on albums rather than tracks, so the station picks random matching albums and plays their tracks.
- **Decade Radio** — Browse decades (1950s, 1960s, ...). Select one to hear music from that era. Like Style, decade metadata is album-level — the station picks random albums from the chosen decade and plays their tracks.

All stations use Plex's PlayQueue API for server-side track selection when available, falling back to direct library queries if needed. Stations do not use sonic similarity — that's used by Sonic Radio (Alt+R), Similar (Alt+M), and Sonic Adventure (Alt+A). Some station features may require Plex Pass.

### Folders (Ctrl+O)

Miller columns style navigation (like macOS Finder). `♪` icon shows the currently playing track.

### Now Playing (Ctrl+N)

Press `Ctrl+N` to cycle between views:
- **Queue**: Current queue or radio tracks
  - Scroll up to see play history (~20 tracks)
  - `Del` removes a track from queue (queue mode only)
  - `Ctrl+S` saves the current queue as a playlist
  - Maximum 500 tracks in the queue
  - Enter/double-click on the currently playing track switches to Now Playing view
- **Now Playing**: Album art, track info, and waveform seekbar
  - Left/Right seeks ±1s, click waveform to seek

Play history is automatically synced to your Plex server, so tracks you play in textamp show up in Plexamp's Recently Played and other Plex clients.

### Playback
| Key | Action |
|-----|--------|
| `Space` | Play/Pause |
| `Ctrl+←` | Previous track |
| `Ctrl+→` | Next track |
| `Shift+←` | Seek backward 10 seconds |
| `Shift+→` | Seek forward 10 seconds |
| `Ctrl+↑` | Volume up |
| `Ctrl+↓` | Volume down |

### General
| Key | Action |
|-----|--------|
| `F5` | Refresh current view (updates cache) |
| `Ctrl+Q` | Quit |
| `Esc` | Cancel / Go back |

## Album Artwork

Album artwork is displayed in the queue view when playing music. Artwork display requires a terminal that supports graphics protocols:

- **Kitty**: Full support via Kitty graphics protocol
- **iTerm2**: Full support via iTerm2 inline images
- **Sixel**: Terminals supporting Sixel graphics
- **Fallback**: Halfblock characters for basic support

## License

MIT
