# textamp

A keyboard-driven Terminal User Interface (TUI) client for Plex Music, written in Rust.

textamp is a specialized, lightweight alternative to Plexamp for power users who live in the terminal. Inspired by musikcube's efficient navigation.

## Features

- **Keyboard-driven**: CUA-style navigation with Ctrl+key and Alt+key shortcuts
- **Hierarchical browsing**: Artist → Albums → Tracks navigation
- **Library views**: Artists, Playlists, Genres, Folders, and Stations
- **Sonic similarity**: Discover similar albums (context-aware)
- **Plexamp Stations**: Library Radio, Deep Cuts, Time Travel, Mood, Decade Radio
- **Create Radio**: Generate radio from any track, album, or artist
- **Sonic Adventure**: Create a sonic bridge between two tracks
- **Album artwork**: Displays cover art in supported terminals (Kitty, iTerm2, Sixel)
- **Search & Filter**: Global search and tabbed filtering (Artists, Album Artists, Albums, Playlists, Tracks, Genres)
- **Folder browsing**: Miller columns view (like macOS Finder)
- **Settings screen**: Configure server, library modes, preferences, and data management
- **Fast startup**: Library data cached to disk for instant display, refreshes in background
- **High-quality playback**: Direct streaming without transcoding

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
| `Tab` | Toggle focus between panels |
| `Enter` / `→` | Select / Drill down / Play |
| `←` / `Backspace` | Go back / Focus left |
| `Page Up/Down` | Scroll by page |
| `Home` / `End` | Jump to top/bottom |
| `A-Z` | Jump to first item starting with letter |

### Categories (Ctrl+key)
| Key | Action |
|-----|--------|
| `Ctrl+A` | Artists (cycles: Artists → Album Artists → Albums) |
| `Ctrl+P` | Playlists (cycles: All → Recently Added → Recent Playlists) |
| `Ctrl+G` | Genres (cycles: Genres → Plex Genres → Moods) |
| `Ctrl+O` | Folders |
| `Ctrl+T` | Stations |

### Views (Ctrl+key)
| Key | Action |
|-----|--------|
| `Ctrl+F` | Search / Filter (tabbed view) |
| `Ctrl+N` | Now Playing (cycles: Queue → Recently Played → Visualizer) |
| `F1` / `?` | Help screen |
| `F2` | Settings |

### Commands (Alt+key)

| Key | Action |
|-----|--------|
| `Alt+R` | Create radio from selection |
| `Alt+E` | Add selection to queue (track or album) |
| `Alt+S` | Similar albums/tracks |
| `Alt+V` | Sonic Adventure (see below) |
| `Alt+P` | Save as playlist (in Now Playing) |
| `Alt+]` | Next track |
| `Alt+[` | Previous track |
| `Alt+O` | Cycle sort order (in Genres) or tabs (in Search) |

### Search / Filter (Ctrl+F)

The unified search screen has tabs for different content types:
- **All**: Global search across Artists, Albums, and Tracks
- **Artists**: Filter artists by name
- **Album Artists**: Filter by album artist tag
- **Albums**: Filter albums by title
- **Playlists**: Filter playlists
- **Tracks**: Filter tracks by title
- **Genres**: Filter genres

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Switch between tabs |
| `←` / `→` | Switch sections in All tab (Artists/Albums/Tracks) |
| `↑` / `↓` | Navigate results |
| `Enter` | Execute search (if query changed) or select result |
| `Esc` | Close search |

Type to enter a search query. Results update when you press Enter. Selecting a track or album plays it while staying in search, so you can continue searching.

### Navigation Flow
- **Artists** (`Ctrl+A`): Press again to cycle between Artists, Album Artists, and Albums views
- **Playlists** (`Ctrl+P`): Press again to cycle between All Playlists, Recently Added albums, and Recent Playlists
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

Additional shortcuts:
- **Alt+O** cycles album sort order: artist, album artist, album title
- Select a genre/mood to see albums, then drill into tracks
- The view remembers your mode when you navigate away and back

### Queue vs Radio

textamp distinguishes between two playback modes:

**Queue** (`Ctrl+N`) - A finite, user-controlled playlist:
- Play an album or playlist to populate the queue
- Queue stops at the end (unless repeat is enabled)
- Navigate and select tracks without disrupting the queue
- Maximum 500 tracks in the queue
- ~20 tracks of play history visible above current tracks

**Radio** (`Alt+R`) - Create radio from selection:
- **Track Radio**: When a track is selected, creates a radio of sonically similar individual tracks (shuffled to avoid album clustering)
- **Album Radio**: When an album is selected, plays similar albums in order (full albums sequentially)
- **Artist Radio**: When an artist is selected, plays that artist's tracks

**Stations** (`Ctrl+T`) - Curated Plex stations:
- Library Radio, Deep Cuts, Time Travel, and more
- Miller columns navigation (Mood Radio has sub-moods, etc.)
- Automatically fetches more tracks as needed

Only one mode is active at a time. Adding items to the queue (Alt+E) while radio is playing converts radio to queue mode.

### Sonic Adventure

Sonic Adventure creates a "sonic bridge" between two tracks - a playlist that transitions smoothly from your start track to your end track using Plex's sonic similarity analysis.

1. Select a start track and press `Alt+V`
2. Navigate to your destination track and press `Alt+V` again
3. Enter the desired length (5-100 tracks)
4. The adventure replaces your queue and starts playing

Tracks can be selected from Browse view or from Search/Filter (Ctrl+F, Tracks tab).

Press `Esc` at any point to cancel adventure mode.

### Similar Albums
The similar albums feature is context-aware:
- When viewing an artist's albums: shows similar to selected album
- When viewing tracks: shows similar to the album containing those tracks
- Otherwise: shows similar to the currently playing track's album

### Stations (Plexamp Radio)
Access Plexamp-style radio stations with `Ctrl+T`:
- **Library Radio** - Shuffles your most-played tracks
- **Deep Cuts Radio** - Plays lesser-known tracks from your library
- **Time Travel Radio** - Chronological journey through your library's history, starting from earliest decades and progressing forward
- **Random Album Radio** - Plays full random albums
- **Style Radio** - Plays by musical style/genre (drill into sub-styles)
- **Mood Radio** - Plays by mood (Aggressive, Atmospheric, etc.)
- **Decade Radio** - Plays music from specific decades

**Miller Columns Navigation**: Categories with sub-stations are marked with `›`. Press `Enter` or `→` to drill into a category (adds a new column). Press `←` or `Backspace` to move focus back to the previous column. Up to three columns are visible at once.

Requires Plex Pass and sonic analysis enabled on your server.

### Folders (Ctrl+O)

Miller columns style navigation (like macOS Finder):
- Three columns visible at once
- `Enter` / `→` to open folder or play track
- `←` / `Backspace` to go back to parent
- `A-Z` to jump to items starting with that letter
- `♪` icon shows the currently playing track

### Now Playing (Ctrl+N)

Press `Ctrl+N` to cycle between views:
- **Queue**: Current queue or radio tracks
  - Scroll up to see play history (~20 tracks)
  - `Del` removes a track from queue (queue mode only)
  - `Alt+O` cycles sort: queue order → by album → shuffled
  - `Alt+P` saves the current queue as a playlist
  - Maximum 500 tracks in the queue
- **Recently Played**: Albums played on this server (synced from Plex)
- **Visualizer**: Audio visualizer with current track info
  - `Alt+O` cycles visualizer styles: Bars, Spectrum, Waveform, Level Meter

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
| `Ctrl+Q` | Quit |
| `Ctrl+C` | Quit |
| `Esc` | Close view / Return to browse |

## Album Artwork

Album artwork is displayed in the queue view when playing music. Artwork display requires a terminal that supports graphics protocols:

- **Kitty**: Full support via Kitty graphics protocol
- **iTerm2**: Full support via iTerm2 inline images
- **Sixel**: Terminals supporting Sixel graphics
- **Fallback**: Halfblock characters for basic support

## License

MIT
