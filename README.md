# textamp

A keyboard-driven terminal-based client for Plex Music.

I use Plex to listen to my music collection but while Plexamp (the offical music client) is pretty good on mobile, I have never liked it on the desktop. This app is designed to offer super-fast navigation of a large music library, and to make my some of my favorite Plex music features (Random Album Radio, Library Radio, Sonic Adventure and Sonic Similarity) available in a fast, keyboard-driven interface with mouse support). It also adds a few new features, such as viewing your artists in a shuffled order, to enable more serendipity and to help surface music that might otherwise be overlooked.  

Additionally, in addition to my main music libray, I have two other audio libraries, one for spoken word, and one for unofficial releases, live recordings, and so forth--a.k.a., bootlegs.  For these libraries, folder-based navigation works better than metadata organization: many files are not tagged properly, and there are often many variations of the same album (different pressings, unofficial remixes) that challenge traditional metadata categories. So, the app is designed to also be fast at navigating the underlying folder structure of libraries, for this kind of content, and to support fast switching between libraries.  To ensure good performance, the app aggressively caches library data and pre-fetches songs in the play queue. 

The main interface paradigm is "Miller columns," which is the same as "column view" in macOS Finder, and close to how iTunes worked before it got all crudded up (with the exception that tracks themselves are in a column in textamp, not in a separate pane).

Some limitations are due to Plex:  for example, it does not distinguish Artist from Album Artist fields, or support the Composer field, etc.

The app was also inspired by other great terminal / text mode music players, such as cmus and Cubic Player (which is the inspiration for the logo and exit message).


### Caching

Library data (artists, albums, playlists, genres, stations) is cached to disk per-library. On startup, cached data loads immediately so you can browse without waiting for your Plex server. Fresh data is fetched in the background and merged automatically.

- **72-hour refresh**: Cache older than 72 hours triggers a background refresh when you navigate to that view.
- **32-day warm cache**: Entries older than 32 days are served immediately from cache but re-fetched in the background when accessed. This applies to both subfolder and artwork caches.
- **Manual refresh**: `F5` forces a refresh of the current view
- **Per-library**: Each library has its own cache, preserved when switching

Subfolder caches (Folders view) are cached lazily when you navigate into them. They are never auto-refreshed or preloaded on startup. At 32+ days, stale entries are served from cache as a "warm cache" and re-fetched in background on access. For smaller libraries where a full crawl is useful, use Settings (F2) > Libraries > Start Subfolder Crawl. Cache progress is visible in Settings (F2) > Libraries. Press `F5` to refresh any folder that seems outdated.

Album artwork is cached to disk. Like subfolders, artwork is never auto-refreshed; at 32+ days, stale images are served immediately and re-fetched in background.

Cache clearing is granular: Settings (F2) > Libraries provides separate options for clearing the library cache, artwork cache, and subfolder cache independently.


### Keyboard Navigation

Every view is navigable without a mouse. `Tab`/`Shift+Tab` moves between categories. `Shift+↓`/`Shift+↑` cycles modes within a category (e.g., Artists → Album Artists → Albums). Arrow keys, `Enter`, and `Backspace` navigate the Miller columns. `Page Up`/`Page Down`, `Home`/`End` work everywhere. But most items are clickable too.

### Alphabetic Jump

Press any letter `A-Z` to jump to the first item starting with that letter. Press `Shift+[letter]` to refine within the current first letter — if you're on an item starting with "A" and press `Shift+N`, you jump to the first "An..." item (like "Andrew").

### Inline Filter

Press `/` to activate a real-time filter on the current column. Type to narrow results instantly. The filter stays active as you drill down, so you can filter artists, select one, then browse their albums without losing the filter.

### Radio Shortcuts

`Ctrl+Alt+L` starts Library Radio instantly. `Ctrl+Alt+R` starts Random Album Radio. `Alt+R` on any selection creates a sonic radio — sonic track radio for similar tracks, sonic album radio for similar albums, sonic artist radio for an artist and similar artists.

### Library Switching

`Ctrl+Alt+S` opens a quick picker to switch between Plex libraries. The switch is instant — cached data for the new library loads immediately while a background refresh runs.

## File Locations

For preference and config file locations, textamp checks for XDG environment variables first, then falls back to platform defaults.

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
| `Ctrl+A` | Artists (cycles: Artists → Albums) |
| `Ctrl+P` | Playlists (Tab to switch: Playlists / Stations) |
| `Ctrl+G` | Genres (Tab to switch: All / Library / Artist / Album / Mood / Style) |
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
| `Alt+R` | Sonic radio from selection |
| `Alt+Q` | Add selection to queue (track or album) |
| `Alt+S` | Shuffle view / queue |
| `Alt+M` | Similar albums/tracks |
| `Alt+B` | Show Album (navigate to track's album) |
| `Alt+G` | Go to Artist (navigate to track's artist) |
| `Alt+A` | Sonic Adventure (see below) |
| `Alt+W` | Save queue/radio as playlist |
| `Alt+C` | Toggle cover art view (album grid with artwork) |
| `Ctrl+Alt+L` | Library Radio (station based on your library) |
| `Ctrl+Alt+R` | Random Album Radio (shuffled albums) |
| `Ctrl+Alt+S` | Quick library switcher |


### Navigation Flow
- **Artists** (`Ctrl+A`): Press again to cycle between Artists and Albums views
- **Playlists** (`Ctrl+P`): Tab bar with Playlists and Stations tabs. Press `Ctrl+P` again to cycle tabs, or press `Tab` to focus the tab bar and use `←`/`→` to switch.
- **Genres** (`Ctrl+G`): Tab bar with All, Library, Artist, Album, Mood, and Style tabs. Press `Ctrl+G` again to cycle tabs, or press `Tab` to focus the tab bar.
- **Folders** (`Ctrl+O`): Miller columns navigation (3 columns visible)

### Genre Types
The Genres view (`Ctrl+G`) provides six tabs for different genre sources:
- **All**: Merged list of all genre types below, with type suffixes for disambiguation (e.g., "Rock (Library)", "Rock (Artist)")
- **Library**: Actual genre tags from your music files (e.g., "Abstract Improvisation", "Post-Punk")
- **Artist**: Plex-generated standardized genre categories at the artist level (e.g., "Rock", "Jazz")
- **Album**: Plex-generated genre categories at the album level
- **Mood**: Plex analysis-based mood metadata (e.g., "Energetic", "Melancholic")
- **Style**: Plex analysis-based style metadata (e.g., "Avant-Garde", "Ambient")

### Queue vs Radio

textamp distinguishes between different playback modes:

**Queue** (`Ctrl+N`) - A finite, user-controlled playlist:
- Play an album or playlist to populate the queue
- Queue stops at the end
- Navigate and select tracks without disrupting the queue
- Maximum 500 tracks in the queue
- ~20 tracks of play history visible above current tracks

**Sonic Radio** (`Alt+R`) - Create radio from selection using sonic similarity:
- **Sonic Track Radio**: When a track is selected, creates a radio of sonically similar individual tracks (shuffled to avoid album clustering)
- **Sonic Album Radio**: When an album is selected, plays similar albums in order (full albums sequentially)
- **Sonic Artist Radio**: When an artist is selected, plays tracks from the artist and similar artists

**Stations** (via `Ctrl+P` Stations tab) - Curated Plex stations:
- Eight station types: Library, Deep Cuts, Time Travel, Random Album, On This Day, Mood, Style, Decade
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

Access stations via the Stations tab under Playlists (`Ctrl+P`, then Tab to switch). Stations use Miller columns — the first five are directly playable, while the last three are categories you drill into to pick a sub-station.

**Directly playable:**

- **Library Radio** — Tracks from your library, weighted by popularity, ratings, and recency via Plex's PlayQueue API.
- **Deep Cuts Radio** — Tracks you haven't played much, using Last.fm popularity data to surface genuinely obscure gems.
- **Time Travel Radio** — A chronological walk through your library starting from its earliest decade. Picks a couple of albums per decade, takes a few tracks from each, then advances forward in time. Wraps around to the beginning when it reaches the end, so it plays indefinitely.
- **Random Album Radio** — Picks a random album and plays it front to back. When it finishes, fetches another random album.
- **On This Day** — Albums from your library that were released on today's date, prioritizing milestone anniversaries. Only appears when your library has matching albums.

**Category stations (drill in to select):**

- **Mood Radio** — Browse moods like Aggressive, Atmospheric, Energetic, Melancholic, etc. Select a mood to hear tracks tagged with it. Mood metadata lives on individual tracks, so filtering is direct.
- **Style Radio** — Browse musical styles like Rock, Jazz, Electronic, etc. Select a style to hear albums in that style. Style metadata is on albums rather than tracks, so the station picks random matching albums and plays their tracks.
- **Decade Radio** — Browse decades (1950s, 1960s, ...). Select one to hear music from that era. Like Style, decade metadata is album-level — the station picks random albums from the chosen decade and plays their tracks.

All stations prefer Plex's PlayQueue API for server-curated track selection, falling back to direct library queries if the server doesn't support it. Stations do not use sonic similarity — that's used by Sonic Radio (Alt+R), Similar (Alt+M), and Sonic Adventure (Alt+A). Some station features may require Plex Pass.

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

## Album Artwork

Album artwork is displayed in the queue view when playing music. Artwork display requires a terminal that supports graphics protocols:

- **Kitty**: Full support via Kitty graphics protocol
- **iTerm2**: Full support via iTerm2 inline images
- **Sixel**: Terminals supporting Sixel graphics
- **Fallback**: Halfblock characters for basic support

## License

MIT
