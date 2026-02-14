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

### Library Switching

`Ctrl+Alt+S` opens a quick picker to switch between Plex libraries. The switch is instant — cached data for the new library loads immediately while a background refresh runs.

## File Locations

For preference and config file locations, textamp checks for XDG environment variables first, then falls back to platform defaults.

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
| `F1` / `?` | Help screen |
| `F2` | Settings |

### Commands (Alt+/ to see available commands)
| Key | Action |
|-----|--------|
| `Ctrl+F` | Search popup (floating dialog) |
| `Alt+E` | Add selection to queue (enqueue) |
| `Alt+V` | Cycle view mode (context-dependent) |
| `Alt+M` | Similar albums/tracks |
| `Alt+G` | Go to Album (navigate to track's album in Library) |
| `Alt+W` | Save queue/radio as playlist |

### Shortcuts (Alt+/ twice to see available shortcuts)
| Key | Action |
|-----|--------|
| `Ctrl+Alt+A` | Play track album (play album of current/highlighted track) |
| `Ctrl+Alt+L` | Library Radio (station based on your library) |
| `Ctrl+Alt+R` | Random Album Radio (shuffled albums) |
| `Ctrl+Alt+S` | Quick library switcher |

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
| `Tab` / `Shift+Tab` | Cycle visualizer tab (Waveform→Spectrum→Spectrogram) |
| `Alt+V` | Cycle visualizer tab |
| `↑` | Focus tab bar (then `←`/`→` to switch, `↓` to return) |
| `←` / `→` | Seek ±1 second |
| `Esc` | Return to Queue view |

### Navigation Flow
- **Library** (`Ctrl+L`): Miller columns with artists, albums, tracks
- **Playlists** (`Ctrl+P`): Tab bar with Playlists and Stations tabs. Press `Tab` to focus the tab bar and use `←`/`→` to switch.
- **Genres** (`Ctrl+G`): Tab bar with All, Library, Artist, Album, Mood, and Style tabs. Press `Tab` to focus the tab bar.
- **Folders** (`Ctrl+O`): Miller columns navigation (3 columns visible)
- **Queue** (`Ctrl+U`): Artwork + station browser (left), track list (right)
- **Now Playing** (`Ctrl+N`): Artwork + track info + visualizer panel with Waveform/Spectrum/Spectrogram tabs

### Genre Types
The Genres view (`Ctrl+G`) provides six tabs for different genre sources:
- **All**: Merged list of all genre types below, with type suffixes for disambiguation (e.g., "Rock (Library)", "Rock (Artist)")
- **Library**: Actual genre tags from your music files (e.g., "Abstract Improvisation", "Post-Punk")
- **Artist**: Plex-generated standardized genre categories at the artist level (e.g., "Rock", "Jazz")
- **Album**: Plex-generated genre categories at the album level
- **Mood**: Plex analysis-based mood metadata (e.g., "Energetic", "Melancholic")
- **Style**: Plex analysis-based style metadata (e.g., "Avant-Garde", "Ambient")

## Playback Modes

textamp has three playback modes. Only one is active at a time. Activating a DJ mode while a station is playing converts the station to a queue; starting a station deactivates any active DJ mode.

### Queue

A finite, user-controlled playlist. Play an album, playlist, or search result to populate the queue. The queue stops at the end unless a DJ mode is active.

- Maximum 500 tracks
- ~20 tracks of play history visible above current position
- `Del` removes a track; `Shift+↑`/`Shift+↓` reorders tracks
- Add to queue with `Alt+E` from any browse context

### Radio (Stations)

Continuous playback from Plex radio stations. Automatically fetches more tracks as the queue runs low.

### Artist Radio

Artist Radio is available in two places:

- **In Library**: Drill into any artist to see "Artist Radio" at the top of the album list (above "All Tracks"). Press Enter to start Plex radio seeded from that artist.
- **Multi-artist blend**: Select "Artist Radio" in the stations panel to open the multi-artist picker. Enter a count (2-12), then search and select artists. Press Tab to launch a blended radio that round-robin interleaves tracks from all selected artists.

## Stations (Plexamp Radio)

The stations panel is accessible from the Now Playing view (`Ctrl+N`). Press `Tab` to switch focus between the track list and the stations panel. Stations are organized into four sections:

### Plex Radio Stations

The top section contains Plex's built-in radio stations. The first five are directly playable, while the last three are categories you drill into via Miller columns.

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

All stations prefer Plex's PlayQueue API for server-curated track selection, falling back to direct library queries if the server doesn't support it. Some station features may require Plex Pass.

### DJ Modes

DJ modes are guest DJ features that automatically insert tracks into your queue while you listen. Toggle a DJ mode on or off by pressing Enter on it in the stations panel. Only one DJ mode can be active at a time. The active mode shows a dot prefix in the panel.

There are two families of DJ modes:

**Interleaving modes** insert a track between each pair of original queue tracks, then let the next original track play. Your queue tracks still play in order, with DJ picks woven in between:

```
original track 1 → DJ pick → original track 2 → DJ pick → original track 3 → ...
```

- **DJ Gemini** — Inserts the most sonically similar track after each original queue track. Uses Plex's sonic analysis to find the nearest neighbor.
- **DJ Twofer** — Inserts another track by the same artist after each original queue track. Skips insertion if the next queue track is already by the same artist, so you don't get three in a row.
- **DJ Stretch** — Inserts a sonic bridge track between each pair of original queue tracks. Finds a track that is sonically similar to both the current and next track, creating a smooth transition. Uses a looser sonic distance than Gemini to find midpoint candidates.

**Continuous modes** insert tracks after every track (including their own previous insertions), so original queue tracks keep getting pushed further down and you only hear DJ picks:

```
original track → DJ pick → DJ pick → DJ pick → DJ pick → ...
```

- **DJ Freeze** — Keeps the mood going with sonically similar tracks. Finds tracks near the current one in Plex's sonic space, maintaining the same energy, tempo, and feel indefinitely.
- **DJ Contempo** — Keeps the mood going with tracks from the same era. Uses the current track's decade to find more music from that time period.
- **DJ Groupie** — Keeps queueing tracks from the current artist and related artists. Uses Plex's related artists data to build a cluster, then picks diverse tracks from across the cluster.

DJ modes require sonic analysis data on your Plex server (Plex Pass feature). Activating a DJ mode while a station is playing converts the station to a queue so the DJ can modify it. Starting a station deactivates any active DJ mode.

### Actions

- **Sonic Adventure** — Opens a self-contained launcher to create a sonic bridge between two tracks. Search for a start track, then an end track, enter the desired length (5-100), and the adventure replaces your queue. Uses Plex's server-side `/computePath` endpoint when available, falling back to a client-side algorithm.
- **Artist Radio** — Opens the multi-artist radio picker (see Artist Radio above).

### Queue Remix

Queue remix tools are one-time operations that process your entire queue at once, inserting new tracks between existing ones. They appear at the bottom of the stations panel. `Ctrl+Z` undoes the last remix operation.

- **Remix: Gemini** — Inserts the most sonically similar track between each pair of queue items.
- **Remix: Twofer** — Inserts a same-artist track between each pair of queue items.
- **Remix: Stretch** — Inserts sonic bridge tracks between each pair of queue items, creating smooth transitions throughout the queue.
- **Remix: Shuffle** — Shuffles the queue. Press again (or `Ctrl+Z`) to restore the original order.

## Sonic Adventure

Sonic Adventure creates a "sonic bridge" between two tracks -- a playlist that transitions smoothly from your start track to your end track using Plex's sonic similarity analysis. When available, it uses the server-side `/computePath` endpoint for the best results.

Access it via the stations panel ("Sonic Adventure" action item), which provides a self-contained UI for searching and selecting start/end tracks and specifying the length.

### Similar Albums/Tracks
The similar feature (`Alt+M`) is context-aware:
- When viewing an artist's albums: shows similar to selected album
- When viewing tracks: shows similar to the album containing those tracks
- Otherwise: shows similar to the currently playing track's album

## Now Playing (Ctrl+N)

Press `Ctrl+N` to cycle between views:
- **Queue**: Left panel shows album artwork and the stations browser; right panel shows the current queue or radio tracks.
  - `Tab` or `←`/`→` switches focus between track list and stations panel
  - Scroll up to see play history (~20 tracks)
  - `Del` removes a track from queue
  - `Shift+↑`/`Shift+↓` moves the selected track up or down in the queue
  - `Ctrl+Z` undoes the last queue remix operation
  - `Alt+W` saves the current queue or radio as a playlist
  - Maximum 500 tracks in the queue
  - Enter/double-click on the currently playing track switches to Now Playing view
- **Now Playing**: Album art, track info, and waveform seekbar
  - Left/Right seeks, click waveform to seek to position

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
