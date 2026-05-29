# textamp

A keyboard-driven Plex Music client for the terminal (ratatui + crossterm).

I use Plex to listen to my music collection but while Plexamp (the
official music client) is pretty good on mobile, I have never liked it
on the desktop. This app is designed to offer super-fast navigation of
a large music library, and to make some of my favorite Plex music
features (Random Album Radio, Library Radio, Sonic Adventure, Sonic
Similarity) available in a fast, keyboard-driven interface with mouse
support. It also adds a few new features, such as viewing your artists
in a shuffled order, to enable more serendipity and to help surface
music that might otherwise be overlooked.

In addition to my main music library, I have two other audio
libraries — one for spoken word, one for unofficial releases / live
recordings / bootlegs. For these libraries, folder-based navigation
works better than metadata organization: many files are not tagged
properly, and there are often many variations of the same album that
challenge traditional metadata categories. So, the app is also
designed to be fast at navigating the underlying folder structure of a
library and to support fast switching between libraries. To ensure
good performance, the app aggressively caches library data and
pre-fetches songs in the play queue.

The main interface paradigm is **Miller columns**, which is the same
as "column view" in macOS Finder, and close to how iTunes worked
before it got all crudded up (with the exception that tracks are in a
column in textamp, not in a separate pane).

Some limitations are due to Plex: it does not distinguish Artist from
Album Artist, or support the Composer field, etc.

The app was inspired by other great terminal / text-mode music players
such as `cmus` and Cubic Player (which is the inspiration for the logo
and exit message).

## Building

```sh
cargo build --release --bin textamp
```

`build.sh` (Linux/macOS) and `build.bat` (Windows) wrap the same
command and offer `--makepackage` and `--clean` helpers. See
`./build.sh --help`.

### Runtime prerequisites

- Any terminal supporting truecolor.
- For album art, a terminal with Kitty / Sixel / iTerm2 graphics is
  recommended; Halfblock / Braille fallback is used otherwise.

An advisory lock at `<state_dir>/textamp.lock` prevents two textamp
instances from fighting over the shared caches; the second invocation
exits immediately with a clear message.

## Caching

Library data (artists, albums, playlists, genres, stations) is cached
to disk per-library. On startup, cached data loads immediately so you
can browse without waiting for your Plex server. Fresh data is fetched
in the background and merged automatically.

- **72-hour refresh**: Cache older than 72 hours triggers a background
  refresh when you navigate to that view.
- **32-day warm cache**: Entries older than 32 days are served
  immediately from cache but re-fetched in the background when
  accessed. This applies to both subfolder and artwork caches.
- **Manual refresh**: `F5` forces a refresh of the current view.
- **Per-library**: Each library has its own cache, preserved when
  switching.

### Folders are not part of the Plex API

Plex does not expose your library's folder hierarchy through its API
the way it exposes Artists / Albums / Tracks / Playlists. So textamp
discovers folders on its own:

- **Top-level folders** are scanned automatically when you first open
  the Folders view.
- **Subfolders** are cached lazily as you drill into them — without
  this, every folder navigation would be a fresh round-trip to the
  server.
- **Manual deep crawl** is available in `Settings (F2) → Cache →
  Start subfolder crawl` for libraries where you want fast,
  always-cached browsing of the full tree (typically: bootlegs / spoken
  word / anything that's better navigated as files than as albums).
- At 32+ days a stale subfolder entry is still served from cache as a
  "warm cache" and re-fetched in background on access.
- `F5` refreshes any folder that seems outdated.

Album artwork is cached to disk. Like subfolders, artwork is never
auto-refreshed; at 32+ days, stale images are served immediately and
re-fetched in background.

Cache clearing is granular: `Settings (F2) → Cache` provides separate
options for clearing the library cache, artwork cache, and subfolder
cache independently.

## Keyboard Navigation

Every view is navigable without a mouse. `Tab` / `Shift+Tab` moves
between categories. `Shift+↓` / `Shift+↑` cycles modes within a
category (e.g., Artists → Album Artists → Albums). Arrow keys, `Enter`,
and `Backspace` navigate the Miller columns. `Page Up` / `Page Down`,
`Home` / `End` work everywhere. Most items are clickable too.

### Alphabetic Jump

Press any letter `A-Z` to jump to the first item starting with that
letter. Press `Shift+[letter]` to refine within the current first
letter — if you're on an item starting with "A" and press `Shift+N`,
you jump to the first "An..." item (like "Andrew").

### Inline Filter

Press `/` to activate a real-time filter on the current column. Type
to narrow results instantly. The filter stays active as you drill
down, so you can filter artists, select one, then browse their albums
without losing the filter.

### Library Switching

`F3` opens a quick picker to switch between Plex libraries. The switch
is instant — cached data for the new library loads immediately while a
background refresh runs.

## Keyboard Shortcuts

### Navigation
| Key | Action |
|-----|--------|
| `↑` / `↓` | Navigate lists |
| `Tab` | Next view (Library→Playlists→Genres→Folders→Queue→Now Playing) |
| `Shift+Tab` | Previous view |
| `Shift+↓` | Cycle modes within category |
| `Shift+↑` | Cycle modes backwards |
| `Enter` / `→` | Select / Drill down / Play |
| `←` / `Backspace` | Go back / Focus left |
| `Page Up/Down` | Scroll by page |
| `Home` / `End` | Jump to top/bottom |
| `A-Z` | Jump to first item starting with letter |

### Categories (Ctrl+key)
| Key | Action |
|-----|--------|
| `Ctrl+L` | Library |
| `Ctrl+P` | Playlists (Tab to switch: Playlists / Stations) |
| `Ctrl+G` | Genres (Tab to switch: All / Library / Artist / Album / Mood / Style) |
| `Ctrl+O` | Folders |
| `Ctrl+U` | Queue (track list with stations panel) |
| `Ctrl+N` | Now Playing (visualizer: waveform/spectrum/spectrogram) |

### Views
| Key | Action |
|-----|--------|
| `F1` / `Ctrl+H` | Help screen |
| `F2` | Settings |

### Commands
| Key | Action |
|-----|--------|
| `?` / `Ctrl+F` | Search popup |
| `Ctrl+E` | Enqueue at top of queue and play |
| `Ctrl+Shift+E` | Enqueue at end of queue |
| `Ctrl+M` | Similar albums/tracks |
| `Ctrl+J` | Jump to album in Library |
| `Ctrl+S` | Sort popup |
| `Ctrl+W` | Save queue/radio as playlist |
| `Ctrl+X` | Clear queue/radio |
| `Ctrl+Z` | Undo last queue remix |
| `Alt+R` | Random Album Radio |
| `Ctrl+Alt+A` | Search Apple Music |
| `Ctrl+Alt+S` | Search Spotify |
| `Ctrl+Alt+Y` | Search YouTube |

### Queue View Keys (`Ctrl+U`)
| Key | Action |
|-----|--------|
| `Tab` | Toggle focus: track list / stations |
| `Del` | Remove track from queue |
| `Shift+↑` / `Shift+↓` | Move selected track up/down in queue |
| `Ctrl+Z` | Undo last queue remix |
| `Enter` on playing track | Open Now Playing view |

### Now Playing View Keys (`Ctrl+N`)
| Key | Action |
|-----|--------|
| `↑` | Focus tab bar (then `←`/`→` to switch, `↓` to return) |
| `←` / `→` | Seek ±1 second |

### Playback
| Key | Action |
|-----|--------|
| `Space` | Play/Pause |
| `<` | Previous track |
| `>` | Next track |
| `Shift+←` | Seek backward 10 seconds |
| `Shift+→` | Seek forward 10 seconds |
| `Ctrl+Shift+↑` | Volume up |
| `Ctrl+Shift+↓` | Volume down |

### General
| Key | Action |
|-----|--------|
| `F5` | Refresh current view (updates cache) |
| `Ctrl+Q` | Quit |
| `Esc` | Cancel / Go back |

## Genres

The Genres view (`Ctrl+G`) provides six tabs reflecting different
genre sources. Some are written by you (file tags), others come from
Plex's own analysis or normalization:

- **All**: Merged list of every genre below, with a type suffix for
  disambiguation (e.g., "Rock (Library)", "Rock (Artist)").
- **Library**: Genre tags **from your music files** (e.g., "Abstract
  Improvisation", "Post-Punk"). These are the strings you've written
  into your audio metadata; Plex passes them through verbatim.
- **Artist** / **Album**: **Plex-generated** genre categories, derived
  by Plex from its own catalogue mapping and shown at the artist or
  album level (e.g., "Rock", "Jazz"). These are normalized into a
  smaller, broader vocabulary than the Library tags.
- **Mood**: **Plex analysis-based** mood metadata (e.g., "Energetic",
  "Melancholic"). Plex computes these server-side; only available
  when Plex has analysed your library.
- **Style**: **Plex analysis-based** style metadata (e.g.,
  "Avant-Garde", "Ambient"). Same source as Mood.

## Artist Aliases

Plex does not distinguish between Artist and Album Artist fields. When
an album's tracks all have the same `original_title` (track-level
artist) that differs from the album artist, textamp treats that name
as an alias. For example, if all tracks on an album credited to
"Robert Pollard" have `original_title` set to "Guided by Voices", then
"Guided by Voices" becomes an alias of "Robert Pollard". Names that
differ only by a leading "The" (e.g. "Ramones" vs "The Ramones") are
normalized to the same identity and do not create aliases.

Aliases propagate throughout the app: the inline filter, search popup,
radio launcher, adventure launcher, and compilation track display all
recognize aliases when matching artists.

## Compilation Detection

textamp detects compilation albums automatically from cached track
data, without per-album API calls. Albums flagged as compilation
candidates by Plex are checked by grouping their tracks by
`original_title` (track-level artist). Albums with tracks by multiple
distinct artists are confirmed as compilations. Artists who appear
only on compilations (no solo albums) are hidden from the main artist
list; the inline filter redirects matches to their "Compilations"
entry instead.

Each artist's album list shows a "Compilations" entry when they have
tracks on multi-artist compilation albums. Drilling in shows the
compilation albums for that artist, with "All Tracks" at the top
showing all their compilation tracks.

## External Search

`Ctrl+Alt+A`, `Ctrl+Alt+S`, and `Ctrl+Alt+Y` open a search in Apple
Music, Spotify, or YouTube based on the currently selected artist,
album, or track. On macOS, Apple Music opens in the Music app; on
other platforms it opens in the browser.

## Playback Modes

textamp has three playback modes. Only one is active at a time.
Activating a DJ mode while a station is playing converts the station
to a queue; starting a station deactivates any active DJ mode.

### Queue

A finite, user-controlled playlist. Play an album, playlist, or search
result to populate the queue. The queue stops at the end unless a DJ
mode is active.

- Maximum 500 tracks
- ~20 tracks of play history visible above current position
- `Del` removes a track; `Shift+↑` / `Shift+↓` reorders tracks
- Add to queue with `Ctrl+E` from any browse context

### Radio (Stations)

Continuous playback from Plex radio stations. Automatically fetches
more tracks as the queue runs low.

### Artist Radio

Artist Radio is available in two places:

- **In Library**: Drill into any artist to see "Artist Radio" at the
  top of the album list (above "All Tracks"). Press Enter to start
  Plex radio seeded from that artist.
- **Multi-artist blend**: Select "Artist Radio" in the stations panel
  to open the multi-artist picker. Enter a count (2-12), then search
  and select artists. Press Tab to launch a blended radio that
  round-robin interleaves tracks from all selected artists.

## Stations (Plexamp Radio)

The stations panel is accessible from the Now Playing view (`Ctrl+N`).
Press `Tab` to switch focus between the track list and the stations
panel. Stations are organized into four sections:

### Plex Radio Stations

The top section contains Plex's built-in radio stations.

**Directly playable:**

- **Library Radio** — Tracks from your library, weighted by
  popularity, ratings, and recency via Plex's PlayQueue API.
- **Deep Cuts Radio** — Tracks you haven't played much, using Last.fm
  popularity data to surface genuinely obscure gems.
- **Time Travel Radio** — A chronological walk through your library
  starting from its earliest decade.
- **Random Album Radio** — Picks a random album and plays it front to
  back. When it finishes, fetches another random album.
- **On This Day** — Albums from your library released on today's
  date, prioritising milestone anniversaries.

**Category stations (drill in to select):**

- **Mood Radio** — Browse moods like Aggressive, Atmospheric,
  Energetic, Melancholic, etc.
- **Style Radio** — Browse musical styles like Rock, Jazz,
  Electronic, etc.
- **Decade Radio** — Browse decades (1950s, 1960s, …).

All stations prefer Plex's PlayQueue API for server-curated track
selection, falling back to direct library queries if the server
doesn't support it. Some station features may require Plex Pass.

### DJ Modes

DJ modes are guest DJ features that automatically insert tracks into
your queue while you listen. Toggle a DJ mode on or off by pressing
Enter on it in the stations panel. Only one DJ mode can be active at
a time. The active mode shows a dot prefix in the panel.

There are two families of DJ modes:

**Interleaving modes** insert a track between each pair of original
queue tracks, then let the next original track play:

- **DJ Gemini** — Most sonically similar track after each original.
- **DJ Twofer** — Same-artist track after each original.
- **DJ Stretch** — Sonic bridge track between each pair.

**Continuous modes** insert tracks after every track:

- **DJ Freeze** — Sonically similar tracks indefinitely.
- **DJ Contempo** — Tracks from the same era.
- **DJ Groupie** — Current artist + related artists.

DJ modes require sonic analysis data on your Plex server (Plex Pass
feature).

### Actions

- **Sonic Adventure** — Self-contained launcher to create a sonic
  bridge between two tracks.
- **Artist Radio** — Multi-artist radio picker.

### Queue Remix

Queue remix tools are one-time operations that process your entire
queue at once, inserting new tracks between existing ones. `Ctrl+Z`
undoes the last remix.

- **Remix: Gemini** — Most sonically similar track between each pair.
- **Remix: Twofer** — Same-artist track between each pair.
- **Remix: Stretch** — Sonic bridge tracks between each pair.
- **Remix: Shuffle** — Shuffles the queue.

## Album Artwork

Album artwork is displayed in the queue view when playing music.
Artwork display requires a terminal that supports graphics protocols:

- **Kitty**: Full support via Kitty graphics protocol
- **iTerm2**: Full support via iTerm2 inline images
- **Sixel**: Terminals supporting Sixel graphics
- **Fallback**: Halfblock characters for basic support

## License

GPL-3.0. See `LICENSE`.
