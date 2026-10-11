# textamp

A terminal music player for Subsonic-compatible servers (including Navidrome),
AudioMuse discovery, WebDAV and local folders. Audio plays on this computer.

## Build and launch

```sh
cargo build --release --bin textamp
./target/release/textamp
```

Use a color terminal. Kitty, Sixel or iTerm2 graphics support improves artwork;
half-block and Braille rendering are also available. The optional macOS
`Textamp.app` launcher opens the adjacent `textamp` binary in a new Ghostty window.
It does not embed a terminal or contain another player.

## Full-screen viewer

**F7** toggles the full-screen visualizer/artwork viewer from any main view.
**Esc** returns to the same view and focus without changing playback. Use
**Left/Right** or **Tab/Shift+Tab** to cycle six visualizers and artwork;
**1–7** selects a mode directly: 1 artwork, 2 waveform, 3 spectrum,
4 spectrogram, 5 vectorscope, 6 landscape, 7 meters. Click a mode
label to select it. The command palette includes **Full-screen Visualizer** and
**Full-screen Artwork**. Missing art shows a placeholder; resizing uses the
existing artwork and visualizer renderers. Live meters/vectorscope samples are
unavailable on AirPlay; cached analysis visualizers remain available.

## AirPlay output (Linux)

Press **F8**, choose **Sound Output** in `:` / Ctrl+P, or open
**F2 -> Textamp -> Display & audio -> Sound output**. All three open the same
picker with the current output, local devices and discovered AirPlay speakers.
F5 refreshes discovery; arrows highlight; Enter or Select applies; Esc or Cancel
closes without changing output. AirPlay discovery uses the existing
`avahi-browse` command and Avahi service. Stop playback before selecting an
output, then play normally. Choose **System default** to return to local output.
Discovery does not interrupt audio;
output selection never silently falls back to local speakers after an error.

Install the current [Music Assistant airplay-cli](https://github.com/music-assistant/airplay-cli)
sender separately from its official release and set `TEXTAMP_AIRPLAY_HELPER` to
the executable path, put the executable named `cliairplay` on PATH, or place it
at `textamp-airplay/cliairplay` beside the Textamp executable. Textamp does not
download or link this GPL-3.0 helper. Its licensing and source
obligations must accompany any separate redistribution. Textamp remains under
the Unlicense. Textamp installs no persistent network service, firewall rule or
pairing credential. The sender uses temporary LAN protocol sockets while playing.

Protocol selection uses the full receiver TXT feature/status flags, including
AirPlay 2 advertised through the RAOP service. It does not infer the protocol
from the service name.

This first version selects one IPv4 destination. It sends 44.1 kHz stereo PCM,
supports local files, WebDAV and Subsonic/Navidrome streams, and forwards volume,
pause/resume and stop. Seeks and track changes reconnect the sender, with startup
buffering. Position follows the sender's scheduled start rather than decoded
bytes written ahead. Live visualizers are unavailable on this output.

The sender supports original AirPlay and AirPlay 2, but Textamp uses unprivileged
NTP timing and does not create persistent pairing credentials. Receivers requiring paired credentials
or PTP timing (which can include HomePods and Apple TVs) report an error and need
explicitly approved setup before they can be supported. Stereo pairs and
synchronized multiroom have not been validated and are not offered here.

## Libraries

Startup opens the last selected library from its cache, falling back to another
saved source if necessary. It never starts playback automatically. With no saved
source, add one in **F2 → Libraries → Add library**.

**F3** is the compact switcher: Enter switches, Esc cancels, F2 manages.
Settings opens a DOS-style setup screen: a menu bar, framed pages, radio buttons,
checkboxes and a key legend, using the active app theme. **F10** focuses the menu;
**Left/Right** selects a menu or page, **Down** enters its contents, and **Tab**
changes panels. **Enter/Space** selects controls; **Esc** backs out.
**:** or **Ctrl+P** opens the command palette from settings.
Every visible control also works with the mouse.

- **Libraries** lists saved sources by type and account. `[Active]` identifies the
  current library. **Up** from the first row returns to the Settings menu.
  Enter opens options for the highlighted library; **Left/Right**
  switches between **General**, **Analysis** (when available), and **Cache**.
- **General** contains **Make active**, rename, connection and removal. Saving a
  connection does not activate another library. Canceling a connection or rename
  form returns to its library setup page. Removal names its scope: a Navidrome
  account includes all its libraries; music and server playlists are kept.
- **Analysis** contains the Sonic checkbox and AudioMuse connection.
- **Textamp → Display & audio** shows artwork rendering and the default cover
  visibility for the active library. Changing either saves it for that library;
  each library restores its own choice when opened. A column's artwork toggle
  also updates only that library's default.
- **Cache** shows metadata and artwork size for that library. Downloaded
  Navidrome images are stored separately for each library. **Clear downloaded
  artwork** removes only that library's image files.
  **Refresh catalog now** shows metadata refresh status. **Refresh catalog
  now** updates metadata and enabled analysis; the same button cancels a running
  refresh. **Clear library cache** asks for confirmation. These actions do not
  change the active library or download audio.
- **Add library** (or **A**) opens a source-type chooser. Local folders have one
  path/name form; servers have one URL/username/password/name form. Add or Save
  checks the source before storing it. A blank password while editing the same
  account keeps the saved password.
- **Textamp** has Themes, Display & audio, Sidebar and Search services pages.
  Themes retain the live ANSI preview; Enter applies a theme. Radio buttons and
  checkboxes save immediately. Artwork cache cleanup is under each library's
  Cache page.
- **About** retains the original ANSI artwork and program information.
- Multiple accounts and music folders are supported. A single-folder account
  has no duplicate “All music” entry. Duplicate connections are rejected.
- Local paths accept `~/Music`. WebDAV uses the exact collection URL; redirects
  are not followed. Use HTTPS outside trusted networks.

Navidrome is the tested OpenSubsonic implementation. Other compatible servers may
lack optional features; unsupported requests report errors rather than using a
different backend.

## Controls and browsing

| Key | Action |
|---|---|
| `:` | Command palette; `:q` Enter quits |
| `/` / `Ctrl+F` | Filter focused column / search |
| `Tab` / `Shift+Tab` | Library ↔ Now Playing; focus panes in split mode |
| `↑↓ ←→`, Enter, Backspace | Select, drill in, drill out |
| `Space`, `<`, `>` | Play/pause, previous, next |
| `Shift+←→` / `Ctrl+Shift+↑↓` | Seek 10 seconds / volume |
| `Ctrl+E` / `Ctrl+Shift+E` | Enqueue / insert next |
| `Ctrl+S` | Save queue as a server playlist |
| `Ctrl+X` / `Ctrl+Z` | Clear queue / undo edit |
| `F1` / `F2` / `,` | Help / Settings |
| `F3` / `F4` / `F5` / `F6` | Switch library / artist bio / refresh / sort |
| `\` / `|` | Scrolling columns / tall split |
| `Ctrl+Q` / `Ctrl+C` | Quit |

Click selects without recentering; click the selected row to drill. Double-click
plays; right-click opens the track menu. Drag reorders queue rows. Clicking or
dragging the waveform or scrubber seeks; streams may rebuffer.

**Search…** sits above **browse**. Highlight it for the theme-colored logo and
press Enter to search. Settings → Textamp → Themes shows the active theme's logo, or previews a highlighted theme.
Search boxes support `AND`, `OR`, and `NOT`, with parentheses and quoted exact
phrases. Adjacent terms imply `AND`; `NOT` binds first, then `AND`, then `OR`.
The “white on black” theme uses pure black backgrounds and 50% white text, with underlining for selected list items.
Textamp → Sidebar controls which sections and system lists are shown.

Local/WebDAV browse lazily, directories first, in natural filename order (`2`
before `10`). Playing a file queues its folder from that point. Search covers
audio filenames in the current folder; `/` also filters directory names. No
whole-library scan is required. Symlinks are omitted. Embedded tags supply
playback metadata and artwork without changing filename order.

Navidrome provides artist/album browsing, tagged album genres, playlists,
favorites, ratings, lyrics and saved queues where supported. Its Folders view is
an artist/album tree, not disk directories. System lists such as recently played
are grouped with browsing, separate from user playlists.

**Compilations:** Multi-artist releases tagged as compilations (or filed under
Various Artists) appear in **Library → Compilations**. An artist's **Compilations**
entry lists the compilation albums they appear on, separately from their normal
albums. Artists who appear only on compilations are kept out of the main artist
list; their music remains available through Compilations and search. Single-artist
collections, such as greatest hits, stay with the performer instead. Ordinary
albums with guest artists are not treated as compilations just for having guests.
In a Compilations list, **All Tracks** includes the complete listed albums.

This grouping uses cached album and track metadata, not AI, and never changes
your music tags. It is rebuilt on cache load and library refresh. Older caches
already support the Various Artists rule; use **F5** in Library to fetch newly
supported compilation tags. Missing performer data is not guessed. Local/WebDAV
keep their normal folder browsing.

Artist biographies use the provider first, then an on-demand English Wikipedia
lookup. Exact names/aliases must match a musical-artist identity in Wikidata;
ambiguous matches are not guessed. Articles show clean text and photos. Arrows
scroll/change photos; B opens the source. Down past the text or Tab focuses
Search Google; Enter activates it. G searches directly in your default browser.
No custom biography sidecars.

Optional Apple Music, Spotify and YouTube searches are in the palette. Music-app
search needs terminal Automation and Accessibility permissions; web search is the
fallback. Textamp does not grant permissions or start playback in those apps.

## Radio and AudioMuse

Radio controls are shared. Local/WebDAV offer Library Radio and Random Album
Radio; bottommost music folders are albums. Discovery samples branches, bounded
to 256 directories and 30 seconds per request, rather than indexing the whole
tree. It is not uniform album sampling.

Navidrome offers random album/artist, deep cuts, time travel, date and genre
stations. Random Artist Radio requires 25+ tracks in the selected library.
**Alt+R** starts Random Album Radio. Sonic Radio continues the current song before
matches; a different selected seed plays first. It is also in the track menu.

Twofer, Contempo and Groupie use catalog metadata. Gemini, Freeze, Stretch,
sonic remixes and Sonic Adventure require the server's sonic extension or
AudioMuse. Starting a station preserves the old queue until usable results arrive;
Stop cancels preparation. Failed/empty results do not replace the queue.

In Settings → Libraries, **I** toggles Sonic features for a server library.
Off hides analysis-dependent controls and cancels pending analysis work without
stopping music. On permits attempts, not a guarantee that analysis is complete.
Folder sources have no sonic toggle.

**M** configures direct AudioMuse access for the selected account/music folder:
URL, username, password and AudioMuse music-server name. Its server-scoped
analysis export (`/api/sync`) is required. The **audiomuse** sidebar offers labels,
tempo, energy, key, analyzed tracks, Describe music and Search lyrics. CLAP and
lyrics searches require their respective indexes. Partial scans yield partial
results. Inferred labels remain separate from genres in file tags.
Describe music uses a compact wrapped prompt that scrolls as you type. When its
sidebar row (or Search lyrics) is highlighted, Right returns to the current
results; Enter opens a new search prompt. Before a search, Right also opens the
prompt. Track
details show all track genre tags and offer Play Track++ to play the selected
track and following tracks.

Results resolve by exact server track IDs, not guessed titles. No playlist is
created unless you save the queue. Textamp does not start scans, alter tags,
change AudioMuse settings or access its database. Disabling Sonic features hides
AudioMuse discovery; its connection can still be managed in Settings.

## Visualizers and caches

Waveform, spectrum, spectrogram, vectorscope, spectral landscape and studio meters
work with every supported library, without AI. Meters show pre-volume PCM RMS,
sample peaks and correlation—not LUFS or true peaks. Pause holds meters. Long
recordings are summarized incrementally rather than retaining whole-track PCM.
Analysis tolerates small undecodable portions as timed gaps; extensive failures
still report an error. This does not alter playback or your files.

Every library type has a persistent metadata cache. Navidrome caches artists,
albums, tracks and playlist metadata. Local/WebDAV libraries index their folder
hierarchy in the background; album track listings load and cache only when opened.

Saved data opens first. Metadata refreshes weekly while a library is in use;
**F5** refreshes the current view. Server system lists query on opening. AudioMuse
shares the weekly/manual policy and fetches changed records.

**Settings → Libraries → Enter** shows that library's on-disk cache size,
including AudioMuse. **Re-scan cache** rebuilds its catalog/tree and enabled
AudioMuse data without switching libraries or interrupting playback. Select it
again to cancel. Failures preserve the last usable snapshot. **Clear library
cache** removes only that library's metadata/analysis; reopen or re-scan to rebuild.
Shared artwork has a separately labelled clear control. Music and sign-ins stay
untouched.

Shared artwork, waveform and spectrogram caches live separately.
Renaming a library does not change its cache identity.
XDG overrides are honored; macOS defaults are `~/Library/Application Support/textamp`
for configuration/private credentials and `~/Library/Caches/textamp` for caches.
Provider snapshots live in `sources/` beneath the cache directory.

## Architecture and checks

`src/ui` renders state; `src/app` owns state, actions and task lifetimes.
`src/app/sources` routes provider operations. `src/navidrome`, `src/audiomuse` and
`src/library` own protocol/file access; `src/media` owns artwork and analysis;
`src/audio` owns local decoding/output. Async results carry operation/library
identity so superseded work cannot replace a newer selection.

The retired Plex implementation is preserved only in [archive](archive/README.md),
not compiled. Non-Plex configuration remains compatible; obsolete Plex settings
are ignored. Personal Plex credential files are not deleted.

```sh
cargo test
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo build --release --bin textamp
```

`cargo run --example navidrome_check` performs read-only checks using saved access;
`-- --catalog` checks catalog loading, `-- --sonic` probes similarity, and
`-- --audio` explicitly exercises muted native playback. `audiomuse_check -- --sync`
refreshes local analysis without starting scans or playback.

Ignored live tests and `tests/fixtures/navidrome_tui_smoke.py` require the dedicated
loopback fixture described in the script. Never aim them at a personal server:
they modify fixture playlists, favorites, ratings, play history and saved queues.

## License

Unlicense. See [LICENSE](LICENSE).
