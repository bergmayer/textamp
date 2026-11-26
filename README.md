# textamp

A terminal music player for Subsonic-compatible servers (including Navidrome),
AudioMuse discovery, WebDAV and local folders. Audio plays on this computer.

## Features that need more than stock Navidrome

A standard Navidrome connection supports catalog browsing, playback, server playlists, favorites, ratings and downloads. Textamp generates waveforms and visualizers locally. The following features depend on what the server has installed, what it has indexed, or which external services you configure.

| Feature | What to configure |
| --- | --- |
| Sonic Mix, Stay Close Mix, sonic queue remixes and Sonic Adventure (Journey) | Enable Sonic features for the library. Textamp can use a server advertising the `sonicSimilarity` extension or a direct connection to AudioMuse with analysis for the selected tracks. The `audiomuseai` Navidrome plugin provides AudioMuse-backed recommendations. |
| Browse analyzed tracks by inferred label, tempo, energy or key | Connect Textamp to AudioMuse under **Settings → Libraries** and select the matching music server. AudioMuse must expose its analysis export at `/api/sync`. |
| Describe music and Search lyrics across the catalog | Configure direct AudioMuse access. Describe music needs the CLAP index. Search lyrics needs the AudioMuse lyrics index. Displaying lyrics for the playing track uses Navidrome’s lyrics API or embedded lyrics for local files. The optional `nd-lyrics` Navidrome plugin can fetch additional lyrics. |
| Opposite Day and Enemies | Install the custom **Distant Rank v1** plugin **on the AudioMuse server** and configure direct AudioMuse access in Textamp. This “far distance” plugin ranks tracks by cosine distance. Opposite Day compares candidates with the queue average. Enemies finds a distant match for each analyzed track. |
| Album Details from local sidecars, with Apple Music editorial fallback | Album Details reads Navidrome’s standard `getAlbumInfo2` response. To serve local reviews, configure the read-only `album-nfo-notes` Navidrome plugin before the `apple-music` metadata agent. Persisting eligible Apple reviews requires a separate exporter with library write access. Paragraphs and source credits belong in the sidecar’s `<review>`; Textamp displays the returned notes. |
| Artist biographies from local metadata files | Install the `artist-nfo-metadata` plugin on Navidrome to read Kodi-style `artist.nfo` files. Artist portraits use the library artwork cache. Compilation-only performers can have portraits and biographies without an artist folder or sidecar. They remain accessible through search and direct links and are excluded from Similar Artists. Textamp also supports server-supplied biographies and a Wikipedia fallback. |
| Search Apple Music | Install Apple Music on the device running Textamp. This command opens the native app independently of the server’s `apple-music` metadata agent. |

For configuration and behavior, see [Radio and AudioMuse](#radio-and-audiomuse) and [library sources](docs/LIBRARY_SOURCES.md).

## Build and launch

```sh
cargo build --release --bin textamp
./target/release/textamp
```

Development and test builds omit debug symbols and reuse `target/debug`, including
Cargo's incremental compilation cache. Use focused tests while iterating, then
run the final verification checks in [AGENTS.md](AGENTS.md). Keep the warm build
caches between changes; remove obsolete artifacts when their combined size
exceeds the 20 GiB budget. Use `CARGO_PROFILE_DEV_DEBUG=2` or
`CARGO_PROFILE_TEST_DEBUG=2` only when debugger symbols are needed.

Go to Random Album opens Album Details for a random catalog album without changing playback: R, Ctrl+Shift+R (or Alt+Shift+R) in the TUI, or ⌘⇧R and Discover → Go to Random Album in the desktop app.

In the TUI, `[` toggles the left sidebar, `]` toggles the queue, and `^` switches
between Library and Now Playing. Tab and Shift+Tab switch panels;
arrows navigate lists. Left/Right adjusts a focused position or volume slider. The footer identifies the focused control. Shift-click, Alt-click or
Track-and-following operations are available in the contextual command palette.
Click a sidebar group name to open it in a column. Groups with 1–512 items, including Playlists, have an arrow to expand or collapse them inline. Playback controls
live in detail panes; column titles open inline filters.

Use a color terminal. Kitty, Sixel or iTerm2 graphics support improves artwork;
half-block and Braille rendering are also available.

## Server playlists

In an online Navidrome library, choose **Add Track to Playlist…** or **Add Album to Playlist…** from the context menu or command palette, then select a server playlist or choose **Create New Playlist…** and enter a name. Albums are appended in disc and track order. In a playlist’s original order, use the pencil on Mac to show checkboxes in either track or album view. In **Group by Album**, click an album’s checkbox to select its playlist tracks, or click the album name to open it and check individual tracks. On iPhone or iPad you can also drag with two fingers to select multiple rows. Choose **Done** to leave editing. In the terminal the pencil enters editing and Space checks a track. Select a track and choose **Move Up in Playlist**, **Move Down in Playlist**, or **Remove from Playlist**. Each command saves to the server. On Mac, Command-click individual tracks or Shift-click a range. In the terminal, use the track selection keys `v` and `V`. On iPhone or iPad, long-press a track, choose **Select Track**, then tap other tracks to select or deselect them. Open the context menu on a selected track or use the command palette. **Add Selected Tracks to Playlist…** appends them in list order, including repeats. Move and Remove apply to every selected playlist entry.

## Downloads

Use Download in track/album details, Download Selection, or Download Play Queue
in the command palette. Downloads appears in the library sidebar and uses normal
track filtering, details, playback and queue actions. Use View Options → Group by
album to browse the downloaded tracks by release. The Downloads list shows its
item count and saved audio size. Pending downloads appear as dimmed rows with
transfer status and an × to cancel each one; in the terminal, x or Delete cancels
the highlighted pending download. Context menus offer Remove Download for saved
items and Cancel Download for pending items; the terminal palette offers the same
actions. Download notifications open this list. Manage
Downloads opens the same destination, with retry, removal and offline commands.
Desktop menus expose the same commands under File → Downloads.

Downloads keep explicit audio copies and artwork in the data directory, separately
from evictable caches. Clearing a download never deletes the source music. Saved
tracks play locally, and album and artist details remain available from saved
metadata. Network-only recommendations show “Unavailable offline” in offline mode. Transfers are bounded to two simultaneous tracks; completed
copies survive restart, while unfinished jobs must be requested again.

## iOS application

The iOS app uses native SwiftUI screens over the shared Rust runtime. It presents
one library column at a time, with Library and Play Queue tabs, a mini-player
that raises Now Playing,
search and command-palette sheets, full-screen Settings, themes and character
artwork. Swipe down to hide the onscreen keyboard and keep editing later.
Queue rows support dragging, swipe deletion and Save Queue as Playlist.
See [iOS build and platform details](docs/IOS.md).

## macOS application

The Mac GUI and TUI sidebars include **Audio Player**. Connect the player in MTP (file transfer) mode. Choose Navidrome
playlists in the column and select **Review Sync…** to inspect additions, updates, removals and matching files. **Apply Sync** transfers the changes. If an MTP transfer is interrupted, Textamp tries to reconnect to the same player and storage up to three times and retries the interrupted file. **On this player…** searches the indexed files across libraries.
The queue has **Next** and **History** tabs. Shuffle keeps the current track first (or the first queued track before playback starts) and shuffles the rest. Playing a later queue entry clears the entries above it. Skipped entries do not count as plays. History retains up to 512 plays across launches. On Mac, the trash button on the right sidebar clears the selected queue tab. Clearing History preserves upcoming music. In the TUI queue, press **h** to switch tabs and **c** in History to clear it.

Audio Player lists the last successful sync with each player on this Mac, including disconnected players. **Clear Sync History** clears those dates and device names.
The player carries its sync selections between Macs configured for the same library.
Synced tracks are organized as `Textamp <library ID>/Artist/Album/01 Title.ext` on the player.
For WebDAV, browse and filter cached folders or open **Synced folders** to review
and uncheck folders synced from the current library. WebDAV keeps the original
paths in its own `Textamp <library ID>` folder. Sync compares the server’s file
versions with the player’s records and reuses matching files, even on a computer with an empty download cache. If WebDAV supplies no usable version, sync downloads a comparison copy on the computer and replaces the player file only when its bytes differ. Navidrome sync uses changes visible in the song catalog to decide which files need comparing. You can browse
other sections of the app while a sync runs.
See [MP3 player setup and limitations](docs/MP3_PLAYERS.md).

The desktop-enabled build is one application with three launch modes:

```sh
~/Applications/Textamp.app/Contents/MacOS/textamp --gui
~/Applications/Textamp.app/Contents/MacOS/textamp --text
~/Applications/Textamp.app/Contents/MacOS/textamp --terminal
```

Finder launch and desktop builds without a mode flag restore the last successful
windowed mode, using Graphical mode for a new profile. GUI and Text are choices under View → Modes and both palettes.
In Now Playing, song, artist and album names open their library details.
⌘⌥T switches between GUI and Text. ⌘⇧T (or Ctrl+Shift+T) also switches modes in the running session. Playback, library requests, browsing and unfinished forms stay alive.
Text mode embeds the existing Ratatui interface through bundled libghostty-vt;
it needs neither a separate Ghostty installation nor another player process.

`--terminal` runs the TUI in the invoking terminal and requires terminal input
and output. It never opens a GPUI window or changes the remembered windowed mode.
Its palette has no windowed-switch command. Contradictory mode flags are rejected.
All modes use the same profile paths, XDG overrides and exclusive process lock;
a second direct invocation reports the held lock. Reopening the running macOS
application focuses its window. New arguments are not forwarded to an existing
process by `open --args`.

The default Cargo build remains a standalone cross-platform TUI. For a native
macOS build use `cargo run --features desktop --bin textamp -- --gui`, or package
with `packaging/macos-desktop/build.sh --release` into `target/desktop/Textamp.app`.

For a separate interactive profile, run `packaging/macos-desktop/build.sh --preview`
and open `target/desktop/Textamp Preview.app`. Preview packaging uses the incremental
development build and shares the feature graph used by desktop tests. Add
`--release` for an optimized Preview. That bundle keeps its own XDG
configuration, credentials, data, state and cache directories, including when
opened from Finder. It does not use the TUI profile.

The desktop uses GPUI Kit controls throughout: Settings pages/groups and forms,
masked credential inputs, list items, menus, tabs, dialogs, breadcrumbs,
metadata descriptions, scrollbars, status bars and transport sliders. See the
GUI-only component policy in [AGENTS.md](AGENTS.md).

Settings (**Textamp → Settings…**, **⌘,**) uses an overlay, with
Libraries, Appearance, Audio, Sidebar, Search and About categories.
Appearance offers Text and GUI in one Mode control; standalone terminals omit it. Theme previews show the highlighted choice; confirmation applies it.
Libraries accepts Subsonic/Navidrome, WebDAV and local sources plus optional
AudioMuse credentials. Opening Settings or saving a connection leaves the active
source and playback unchanged. **F10**, Up/Down selects a category; Enter/Right
enters its controls. Tab moves between controls; **⌘S** saves a focused connection
form. Saved connections also have explicit Edit Connection and Manage controls.

**⇧⌘P** or the **⌘** toolbar button opens the shared command palette. **Esc**
dismisses it immediately, including with a query. Its Zed-style search-first
layout offers fuzzy character highlighting, trailing Mac shortcut hints, recent
commands and a Run/Return footer. **⌘1 / ⌘3** selects Library / Now Playing. Single-click selects or opens children/details without
playing or recentering the clicked column; arrows navigate the hierarchy; Return opens the selected item like Right Arrow;
**⌘[** closes the current column; each column header also has a small close icon
that closes that column and its descendants;
double-click plays. Type a letter to jump within a column (Shift uses the second
letter within the current first-letter group; # and % address numbers and symbols). Fast scrolling temporarily shows
letter shortcuts. Album/track previews, with explicit play/queue actions, follow
the last open Miller column. **⌘-click** and **Shift-click** select multiple tracks;
right-click targets the clicked item and preserves an appropriate existing group.
Artist columns start with All Tracks. Album pages show cover art, metadata and tracks, with Artist Biography and Album Info on either side of Play. Track Details includes available lyrics and track-specific artwork. In the terminal, Left and Right choose playback controls, Down selects a similar track, and Page Up and Page Down scroll through the lyrics. Similar Albums shows recommended releases in a cover grid beneath the tracks. Similar Artists follows with portraits of artists that Navidrome associates with the album artist. Artist album lists show a centered portrait and biography excerpt above the albums, with a Similar Artists grid below.

Library and Now Playing share one window by default. Click empty space in the transport bar to open Now Playing or return to the library. Click transport metadata to open Now Playing. On Mac, click the cover to enter mini mode and click it again to return. The Mac sidebar’s top-level icons stay accessible beside the player. The left sidebar has a visibility toggle. Next and History on the right open the queue to that view. Click the selected icon again to close it. Search, Settings, the command palette and Radio sit below Next and History on the right. Click Radio to start the selected station or hold it to choose a station for future clicks. Playback controls sit in the transport bar.
A small chevron collapses the player back to the bottom; library selection and
scroll positions stay in place. The artwork header and queue remain resizable;
transport controls keep their horizontal alignment throughout the transition.
Column headers start with a capital letter and show the full title when it fits. In narrower columns they
drop the “albums —” or “tracks —” prefix before applying middle ellipsis.
Motion follows the system Reduce Motion setting. Artwork and track
information sit above the visualization; its six tabs select waveform, spectrum, spectrogram, vectorscope, landscape or studio meters. Artwork remains visible beside the track information. On Mac, Next and History open in their own resizable sidebar alongside either Library or Now Playing. The sheet slides up from the edge-to-edge transport, which also holds the navigation and status controls. Opening Now Playing from a focused library column keeps that column in the sidebar. Back (⌘[) moves up one level. Selecting an icon expands the Library sidebar in place. Choose a destination to browse one column at a time alongside Now Playing. Closing that column returns the sidebar to its root. Dismissing it restores the full library layout. The TUI uses ‹ or Backspace to step up in the single Library column. Queue Type, Queue Tools, and Clear sit at the foot of the right sidebar. Double-click a History track to replay it, or open its context menu for Play Next and Track Details. In the terminal, use Enter, +, and Right for those actions. Undo (⌘Z) and
Save Queue as Playlist are available in the menu bar. Queue Tools also opens Save Queue as Playlist. Clear Queue is undoable
without restarting playback. The Queue Type dropdown selects Manual Queue, Continue with Related Tracks, or a radio station; its active indicator marks a non-manual queue.
Stations replace the queue; continuation preserves it and starts from its final track.
Radio and related-track modes append a batch when the second-to-last track starts.
Auto-Mix overlaps adjacent songs with equal-power fades.
Lyrics uses the active Navidrome connection to fetch structured lyrics on demand. Follow Playback keeps the current timed line in view as the song plays or seeks. The graphical apps enlarge and tint that line. The terminal uses bold accent text. Turn off Follow Playback to read freely. Multiple entries offer a language choice. Local files use embedded lyrics. The app does not write lyrics to music files or its library cache. When no lyrics are available, an explicit web search remains available.
Artist, album and genre names in
Now Playing navigate to their Library destinations. The bottom transport shows
the current cover. Drag queue entries to reorder them, including selected
groups; Shift–Up/Down reorders with the keyboard. The current playing occurrence
is preserved. Seek and volume use stock sliders; audio processing and playback
commands remain in the shared app layer. Radio stations refill as music plays,
independently of the original queue length. Queue edits apply once and support Undo.

Click a GUI column title to filter that column, independently of Search. Long column titles use a center ellipsis and begin scrolling after ten seconds.
Click it to show or hide the fixed filter field; `/` focuses it and Escape clears it. The field fills the column
and supports normal text selection, paste and text undo/redo. Filtering retains raw row identities for selection and context menus. Album filters match artist names as well as titles.
View Options uses Kit checkboxes and supports grouped, empty and root playlists.
Track previews fall back to album artwork, independently of row-cover visibility.
Track and album details include similar music from other artists. The Library
path appears in the title bar, without a separate breadcrumb strip.

GUI meters show labeled dBFS segments, 300 ms power integration, instantaneous
sample-peak attack with a 20 dB/s display release, 1.5-second peak holds and
measured stereo correlation. The blue RMS history sits below the stereo bars
with its own 12-second time range and dBFS scale. These are sampled pre-volume PCM levels, not LUFS,
true-peak measurements or output-device loudness.

The visualizer row's expand icon (or F7) shows just the visualizer; Escape restores
the Now Playing layout. The transport tile raises the other window when using separate windows.
Mac uses the system UI font. **⌃⌘F** toggles the native Mac window's full-screen mode.

Library is a sidebar heading with Artists, Albums and Tracks destinations.
File → Create New Playlist… creates an empty playlist on the active Navidrome server. The same command is available under File in the command palette.

**File → Export Playlist** saves the open playlist as UTF-8 M3U8 or CSV. **Export Queue** saves the current playback order. **File → Import Playlist…** creates a new playlist on the connected Navidrome server, named after the selected file. Choose an M3U, M3U8 or CSV file. M3U entries match library file paths. CSV accepts a Path or Title column and can use Artist and Album to distinguish tracks. Order and repeated tracks are preserved. An unmatched or ambiguous entry stops the import before anything is saved. The terminal uses a themed file picker. Mac, iPhone and iPad use their system file pickers.

Individual star ratings are under **Edit → Track Rating** in the command palette and the Mac menu bar.

Settings → Sidebar follows the sidebar’s groups, labels and order, including the
Albums subitems. Its visibility checkboxes retain hidden destinations so they can
be restored. Status, loading and error messages use an opaque, fixed-height strip
below the transport. The focused column’s displayed-row count appears there only;
column headers contain the title and relevant controls.
Radio Stations is available in the queue’s Radio menu, Discover and the command
palette. Artist Mix and seed-based stations use a single chooser.
In text mode, F6 or the column’s ≡ button opens View → View Options in the command palette. The Mac menu bar uses the same submenu. Unavailable options appear dimmed.

Single-album pages keep tracks in release order. Other columns’ View Options buttons offer the same sort modes, direction,
grouping and artwork options as the TUI for that kind of column. Show artwork
defaults on and is remembered by column type across both presentations; playlists
also have Group by album. Album columns default to **Grid**, including grouped downloads and playlists.
Choose **List** in View Options for compact rows. In the Mac grid, Left and Right
move between covers, Up and Down move by row, and Return opens the selected album. On an artist’s page, the web search button appears below the portrait
when a biography is unavailable. When a biography is available, open **…more**
to find the same button in the reader. In the terminal, press Up from the first row and Enter to open the biography. The album page offers the biography button
while availability is unknown and switches to web search after an unsuccessful lookup. Choose Google or DuckDuckGo in Search settings.
Album rows without artwork show a generic cover in the same square slot.
Settings → Cache & Scanning shows per-library disk use, re-scan/cancel and clear
controls. Both frontends share cached startup, weekly refresh and cache jobs.

View → Reset View restores panel sizes. Artist, album and genre metadata links
open the corresponding library view and reveal the selected entry in its column.
Keyboard selection and scrolling are immediate; child inspection waits for a
250 ms pause during rapid navigation. Existing child columns, details and artwork
stay visible throughout the pause and any replacement listing request. Queue Return and Play use the existing
queue position, including repeated occurrences of a track.

Journey uses explicit start/end choices and a maximum track count. Preview Journey
builds a route without changing playback; Play Journey then replaces the queue. Paths preserve the endpoints
and use each artist once, so the result can be shorter than the maximum.

Global search ranks exact and near-exact titles across result types. Albums precede equally close track matches. Playlists sits directly below Albums and has a name filter. Open Radio to browse its station column, or use its disclosure arrow to show stations inline. Use its field on desktop, tap the column title on iOS, or press `/` in the terminal. Add to Playlist remembers the last destination separately for each library and puts it first next time.

Playlists live in the left sidebar. Pausing on one during keyboard navigation
opens it after 250 ms while keeping focus in the sidebar; Right or Return enters
its tracks. Selecting one opens its tracks as the first
visible Miller column, with details immediately to its right. Playlist cover art
uses the focused track column and loads the visible tracks' covers.
Global command-palette actions are also available in the native menu bar,
including volume/seek, sorting, radio/queue-edit tools, provider queue commands and the
current radio-station hierarchy. Bare letter keys jump within the focused column.

Textamp recalculates station availability from each Navidrome catalog when it loads or refreshes. Mood, genre and decade choices require two distinct matching tracks. AudioMuse mood choices are checked against the selected library when Mood Mix opens. Local and WebDAV libraries use playable files found during folder scanning and browsing. A WebDAV album folder can supply artwork in `cover.jpg`; embedded track artwork takes precedence during playback.

Radio in the sidebar, Discover → Radio, and the queue’s Radio button all open the same Radio columns in the library workspace. Radio lists stations and station categories. One-time edits such as Shuffle, Twofer and Stretch live in Queue Tools. Artist Mix opens an artist chooser. On iPhone and iPad, search by name, add or remove artists, then tap Play Mix. Stations requiring an input open their controls immediately in the detail pane; selecting them does not start playback. Discover includes Radio, AI, Journey, Library Mix, random album playback and Go to Random Album.
The fast-scroll alphabet index occupies a separate fitted strip, without
covering track names. Library navigation dismisses discovery popups and their
return targets.

Native dialogs present search and adjacent result details, similar/related music,
biographies, lyrics/analysis, sorting, output
selection, library management and confirmations. Capability checks, cancellation,
request identities and stale-response protection remain shared with the TUI.
The brushed aluminum theme frames the Mac workspace in textured silver with an inset border. Mobile uses a narrower rim. The terminal uses flat silver surfaces and blue selections.
The desktop remains experimental. See [desktop architecture and interactions](docs/DESKTOP.md)
and [runtime boundaries](docs/RUNTIME.md). The complete TUI remains the default frontend.

## Full-screen viewer

**F7** opens the full-screen visualizer from any main view in both frontends.
**Esc** returns to the same view and focus without changing playback. Use
**Left/Right** or **Tab/Shift+Tab** to cycle six visualizers and artwork;
**1–7** selects a mode directly: 1 artwork, 2 waveform, 3 spectrum,
4 spectrogram, 5 vectorscope, 6 landscape, 7 meters. Click a mode
label to select it. The command palette includes **Full-screen Visualizer** and
**Full-screen Artwork**. Missing art shows a placeholder; resizing uses the
existing artwork and visualizer renderers.

Desktop Navidrome startup combines all music folders for the selected account.
Switch Library labels individual folders as narrower scopes. Settings lists one
connection per account and names its included folders. Settings → About displays
the original colored ANSI logo.

Textamp uses the system audio output. Select speakers or headphones in your OS
sound settings; playback volume remains in the transport bar.

⌘1 and ⌘3 show Library and Now Playing; ⌘2 toggles Play Queue. Separate windows are an optional saved
choice in the Window menu and command palette. Both share one playback engine
and queue. Shrinking Now Playing below 720 points wide or 520 points tall shows a
mini-player with square artwork, title, artist, playback buttons and a seek slider;
expanding restores metadata and visualizers. Play Queue is independent: its transport
button opens a rising, resizable panel beside Library or Now Playing. View Options stays a fixed size.
Recently Added offers Date Added sorting to restore the server’s newest-first
order after another sort; Oldest First reverses it.
Left from the first browser column focuses the sidebar; Up/Down moves through its
visible destinations and disclosure headings. Enter opens a destination. Arrow keys move within grids and between existing columns;
Left collapses its section. Keyboard selections scroll into view.

## Terminal layout

The terminal sidebar uses the same library groups as the desktop: Artists, Albums,
Folders, Genres, Decades, AI and playlists. Enter toggles a disclosure; Right expands
or enters it and Left collapses it. Click a triangle to expand or collapse its group.

Press **⌘2** (windowed), **Ctrl+2**, **Ctrl+U**, or **|** to show or hide Play Queue on the right;
Left returns to the workspace. Queue Type and Queue Tools are icon menus on the right of the transport. Up from the first track reaches Queue Type; Right reaches Queue
Tools. Enter opens the contextual menu. Queue tools
include shuffle, repeat, Twins, Twofer, Stretch, Doppelganger, Opposite Day, Enemies, save, clear, undo,
Queue Type and Auto-Mix. The queue heading identifies the active station or continuation mode.

Now Playing places the visualizer between the artwork and the transport. Settings has a category pane and a content pane; About displays the
original colored ANSI logo. Both frontends use the system's audio output.

Terminal columns share the available width equally and scroll horizontally when they
reach their minimum width. Shift+wheel pans the columns; opening a column reveals it.
Press Up at the first item to focus its header, Left/Right to choose a control, and
Enter to activate it. Clicking a column title or pressing Enter on its header title
opens the filter; Esc or its × closes it. The header provides Play, Add to Queue, View
Options and Close where available. Option/Alt+Play inserts next. Alt+Left and the
footer’s back arrow return from library jumps.

Journey uses the same explicit Start track, End track, Maximum tracks, Preview Journey
and Play Journey workflow in both frontends. Each endpoint is its own search field. Tab
reaches every field and command, Ctrl+Enter previews, and selecting a result never
starts playback. In the terminal, Alt+S/Alt+E selects the Start/End field, Alt+M selects
Maximum tracks, and Alt+W swaps the endpoints. Search and Similar results open the
actual library detail destination; Alt+Left returns to the previous location.

## Libraries

Startup opens the last selected library from its cache, falling back to another
saved source if necessary. It never starts playback automatically. With no saved
source, add one in **F2 → Libraries → Add library**.

**F3** is the compact switcher: Enter switches, Esc cancels, F2 manages.
Settings has a category list and one content pane. **Up/Down** chooses a category;
**Enter**, **Right**, or **Tab** enters its controls. **Tab / Shift+Tab** switches
between the two panes; **Left / Esc** returns to categories, and **Esc** there
closes settings. **F10** focuses categories; **1–7** jumps to a category while
that list has focus. Narrow terminals show one pane at a time.
**Enter / Space** activates a control. Click a checkbox or radio button to change
it; click a theme to highlight and preview it; click the highlighted theme again, press **Enter**, or choose **Apply theme** to save it. Library rows open
on a second click. **:** or **Ctrl+P** opens the command palette. Available commands chosen from the palette are promoted for ten subsequent openings and remain in their normal submenus; this history persists across launches. Available selection commands appear directly before the menu groups. Other groups match the menu bar. Typing searches all commands, including their parent menus. In the TUI, Enter/Right enters a submenu and Left returns.

- **Libraries** lists saved sources by type and account. `[Active]` identifies the
  current library. **Up** from the first row returns to categories.
  Enter opens options for the highlighted library; **Left/Right**
  switches between **General**, **Analysis** (when available), and **Cache**.
- **General** contains **Make active**, rename, connection, re-scan and removal. Saving a
  connection does not activate another library. Canceling a connection or rename
  form returns to its library setup page. Removal names its scope: a Navidrome
  account includes all its libraries; music and server playlists are kept.
- **Analysis** contains the Sonic checkbox and AudioMuse connection.
- **Artwork** selects Images, ANSI half blocks or Braille for the active library.
  Graphical and Text mode retain separate rendering choices. External terminals
  without an image protocol use half blocks without changing the saved choice.
  Artwork encoding follows the library when switching between GUI and text mode.
  Column View Options remember artwork visibility by column type, shared between
  presentations; artwork is on by default.
- **Cache** shows metadata and artwork size for that library. Downloaded
  Navidrome catalog and playlist album images share a 2 GiB cache per library.
  Background fills stop at capacity; interactive requests evict least recently used
  images. Completed scans remove previously indexed covers that no catalog album or playlist
  references. Catalog metadata and visited folder listings each have a 512 MiB
  budget shared across libraries. After 15 minutes a saved folder listing opens
  immediately and refreshes in the background.
  **Clear downloaded artwork** removes only that library's image files.
  **Re-scan library now** updates metadata and enabled analysis; F5 or the same
  button cancels a running scan. Its status is visible below the library list.
  **Clear metadata cache** asks for confirmation. These actions do not
  change the active library or download audio.
- **Add library** (or **A**) opens a source-type chooser. Local folders have one
  path/name form. Music-server forms also include optional AudioMuse settings.
  Add or Save checks the connections and starts a metadata scan. A blank password
  while editing the same
  account keeps the saved password.
- **Appearance** previews a theme with a sample music list; Enter applies it.
- **Audio** contains sound output and server streaming quality. **Sidebar** controls
  visible browse entries; **Search** controls external search services.
  Changes save automatically. Artwork settings apply to the active library.
- **About** shows version, project and terminal information.
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
| `/` / `Ctrl+F` | Filter focused column / search. Empty search shows the last 12 results chosen in this library. Select one to open it directly. |
| `Tab` / `Shift+Tab` | Next / previous pane |
| `⌘2` / `Ctrl+2` / `Ctrl+U` / `\|` | Show or hide the queue |
| `Alt+T` | Focus transport controls |
| `Alt+A` | Focus visible artist names; arrows select, Enter opens the artist’s albums |
| `↑↓ ←→`, Enter, Backspace | Select, drill in, drill out |
| `Space`, `<`, `>` | Play/pause, previous, next |
| `Shift+←→` / `Ctrl+Shift+↑↓` | Seek 10 seconds / volume |
| `Ctrl+E` / `Ctrl+Shift+E` | Enqueue / insert next |
| `Ctrl+S` | Save queue as a server playlist |
| `Ctrl+X` / `Ctrl+Z` | Clear queue / undo edit |
| `F1` / `F2` / `,` | Help / Settings |
| `F3` / `F4` / `F5` / `F6` | Switch library / Artist Biography / refresh / sort |
| `Alt+←` | Back from a library jump |
| `Ctrl+Q` / `Ctrl+C` | Quit |

Click selects without recentering; click the selected row to drill. Double-click
plays; right-click opens the track menu. Drag reorders queue rows. Clicking or
dragging the waveform or scrubber seeks. The first seek in a remote track may
rebuffer while preparing a temporary file; later seeks reuse it.

**Search…** sits above **browse**. Highlight it for the theme-colored logo and
press Enter to search. Settings → Appearance previews a highlighted theme using a sample music list.
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
list; their music remains available through Compilations and search. Various
Artists releases remain compilations even when only one performer's tracks are
present. Single-artist collections, such as greatest hits, stay with the performer
when the album credit and complete track list agree. Ordinary
albums with guest artists are not treated as compilations just for having guests.
In a Compilations list, **All Tracks** includes the complete listed albums.

This grouping uses cached album and track metadata, not AI, and never changes
your music tags. It is rebuilt on cache load and library refresh. Older caches
already support the Various Artists rule; use **F5** in Library to fetch newly
supported compilation tags. Missing performer data is not guessed. Local/WebDAV
keep their normal folder browsing.

Artist biographies use Navidrome's configured metadata agents first, with an
eight-second artist-info request limit. Missing biographies, server errors and
timeouts fall back to an on-demand English Wikipedia lookup. Cached or returned
MusicBrainz artist IDs, when available, constrain the Wikidata identity;
MusicBrainz's Wikidata link can resolve an artist whose name has changed.
Without an ID, exact names/aliases must match one musical artist in Wikidata;
ambiguous matches are not guessed, including when another match has no English
article. Conflicting server identities are withheld; Wikipedia retains its strict
identity checks. An image failure does not discard loaded biography text. If both biography
sources are unavailable, the pane says “No biography available”.
While a biography loads, the artist column offers web search. The preview and its …more button appear when usable text arrives. Textamp keeps up to 32 completed lookups in memory for the active library. A result with prose is reusable for 24 hours and a miss for one hour. Refresh Library clears those results.

Navidrome supports biographies through optional external agents, but their
availability depends on server configuration and coverage. Articles show clean text and photos. Arrows
scroll/change photos; B opens the source. Down past the text or Tab focuses
Search Google; Enter activates it. G searches directly in your default browser.
No custom biography sidecars.

Apple Music and YouTube searches are enabled by default; Spotify can be enabled
in Search settings. The iOS overflow menu offers app-only searches. External search
uses visible details first, then the active artist or album column, then the playing
track; commands are disabled without a music subject. All enabled services are
available in the contextual palette. Search Apple Music uses ⌘⇧A on Apple keyboards or Ctrl+Shift+A in the desktop and terminal interfaces. On iOS, for a selected song, album, or artist, Textamp checks Apple’s U.S. catalog for an exact match and opens its link in Music. An ambiguous or missing match, or an unavailable catalog, falls back to the original search link. Music controls whether it displays search results after launch.

## Radio and AudioMuse

Each radio station needs playable music from at least 20 distinct albums credited to 20 distinct album artists. A session queues at most one track from each album, including refills. Starting a new session makes those albums eligible again. Browsing albums and decades has no minimum.

Textamp rebuilds catalog station choices when it loads a library or finishes a refresh. Local folder scans use embedded album and album-artist tags from sampled files. For WebDAV, Textamp learns those credits from files opened for playback in the active library. Artist and sonic mixes check the recommendations for the chosen seed before starting playback.
**Alt+R** starts Play Random Albums. Sonic Mix continues the current song before
matches; a different selected seed plays first. It is also in the track menu.

The Twofer queue edit uses catalog metadata. Sonic remixes and Journey require
the server's sonic extension or AudioMuse. Starting a station preserves the old queue until usable results arrive;
Stop cancels preparation. Failed/empty results do not replace the queue.


The Navidrome stations use these selection rules:

| Station | What it plays |
| --- | --- |
| Play Random Albums | One song per randomly chosen album. |
| Random Artist Mix | A mix drawn from artists with at least 25 distinct catalog tracks each. |
| Library Mix | Up to 100 shuffled tracks from the active library per batch. |
| Deep Cuts Mix | Shuffled tracks at or below the library's 25th-percentile play count; ties can include more than a quarter. |
| Time Travel Mix | Successive release decades, oldest to newest, with shuffled tracks in each batch. |
| On This Day | Tracks from albums released on today's month/day, regardless of year. |
| Mood Mix | Catalog moods use matching albums. AudioMuse moods use analyzed tracks. |
| Style / Genre Mix | Shuffles songs from albums with the chosen genre. |
| Decade Mix | Tracks from albums released in the chosen decade. |
| Artist Mix | Choose one or more artists. Combines their tracks with related recommendations. |
| Sonic Mix | Recommendations follow the currently playing track at each refill. |
| Stay Close Mix | Recommendations keep using the original chosen track as their seed. |

The Artist Mix control in an artist’s column header starts a mix for that artist. In Radio, choose Artist Mix to select up to 12 artists before starting playback.
Station choices follow
the active library's capabilities and sonic settings. Metadata stations use the
cached catalog; unavailable tags or play counts produce a visible error.

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
On iPhone and iPad, swipe left on a Describe Music history entry or hold it and choose Remove from History. This removes the saved search and its results from the current library.

Describe music uses a compact wrapped prompt that scrolls as you type. When its
sidebar row (or Search lyrics) is highlighted, Right returns to the current
results; Enter opens a new search prompt. Before a search, Right also opens the
prompt. Track
details show all track genre tags. Track context menus offer playback and queue operations for the selected track and following tracks.

Results resolve by exact server track IDs, not guessed titles. No playlist is
created unless you save the queue. Textamp does not start scans, alter tags,
change AudioMuse settings or access its database. Disabling Sonic features hides
AudioMuse discovery; its connection can still be managed in Settings.

**Opposite Day** and **Enemies** require **Distant Rank v1**, a custom plugin
that must be installed separately on the AudioMuse server. Textamp uses the
saved AudioMuse connection and limits replacements to the active Navidrome
library. The first edit can take longer while AudioMuse loads its ranking index.

## Visualizers and caches

Waveform, spectrum, spectrogram, vectorscope, spectral landscape and studio meters
work with every supported library, without AI. Meters show pre-volume PCM RMS,
sample peaks and correlation—not LUFS or true peaks. Pause holds meters. Long
recordings are summarized incrementally rather than retaining whole-track PCM.
Analysis tolerates small undecodable portions as timed gaps; extensive failures
still report an error. This does not alter playback or your files.

Every library type has a persistent metadata cache. Navidrome caches artists,
albums, tracks, playlist metadata and ordered playlist contents (including repeated tracks).
The cached catalog opens immediately. Interrupted fills resume in the background
when the library reopens. After a network catalog
refresh, a cancellable background scan prepares ordered playlists and album
covers, including playlist albums outside the selected music folder. It reuses
unchanged playlist revisions and saved images. **Re-scan library now** also
checks playlist contents with the server. Up to eight playlist requests and
four artwork requests run concurrently. Local and WebDAV scans list the root
and two directory levels below it. Deeper folders and track listings load when
opened. Neither type of scan downloads audio.

Saved data opens first. Metadata refreshes weekly while a library is in use;
**F5** refreshes the current view. Server system lists query on opening. AudioMuse
shares the weekly/manual policy and fetches changed records. Folder scans and
changed AudioMuse metadata batches use up to four concurrent requests.

The Subsonic / Navidrome connection form includes optional AudioMuse fields,
so both connections can be checked and saved together. Leave the AudioMuse URL
blank to skip it or retain an existing connection.
New libraries start their metadata scan in the background.

In **Settings → Libraries**, use **A / Add library**, **Enter / Manage**, or
**F5 / Re-scan** for the selected library. F5 cancels a running scan. Its status
appears below the list, without changing the active library or playback.

**Settings → Libraries → Manage → Cache** shows the library’s on-disk metadata, analysis and artwork sizes. For Navidrome it also shows when the last full catalog sync completed. **Re-scan library now** rebuilds its catalog/tree and enabled
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

Modal selection, file preparation, artwork fetching and navigation settling,
scrobble reporting, use explicit enum states. Opening
a modal replaces the previous one; a confirmation can suspend a library dialog
and restore it on dismissal. Replacing file preparation drops its task lease,
and playback identities reject late completions.

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

Textamp source is released under the Unlicense. Dependencies retain their own licenses.

Library jumps retain six previous library views. Backspace, the Back button,
or Alt+Left restores the previous view, including its selections and scroll
positions. Ordinary column browsing does not add history entries. Enter on a
library item opens it without playback; double-click and explicit playback
commands remain available.

In the TUI, Tab and Shift-Tab move between panels; arrows navigate inside each
panel. In the transport panel, Up/Down selects controls and Left/Right adjusts
the focused slider. Settings opens in a centered popover that adapts to the terminal size.

Settings values, command variants, queue tools and detail metadata use shared
application definitions. Album artwork cards put the title and year beside small images and below
large images in both presentations.
The terminal footer describes the focused control ahead of background activity. Track-and-following operations appear in the contextual command palette.

The windowed Text and graphical modes retain pane geometry when switching, with
a monospace font in Text mode and the system font in graphical mode. Both show
elapsed and remaining time beneath the transport scrubber and retain the small
artwork thumbnail in Library and Now Playing. Embedded Text uses graphical visualizers with
graphics artwork and character visualizers with half-block or braille artwork.

On Unix, Cargo test executables run with a temporary XDG profile through the repository runner. Normal Cargo launches retain the selected personal profile. Direct native checks must set separate configuration, data, cache and state paths.

For repeatable large-catalog and cached-startup measurements, see
[Measuring responsiveness](docs/RUNTIME.md#measuring-responsiveness).
F5 retries failed related music in the current detail pane while keeping the
library destination and its tracks visible.

In Discover, Command–Option–R opens Radio, Command–Option–L starts Library Mix, Command–Option–A starts Play Random Albums, and Command–Option–J opens Journey. Terminal bindings use Control–Alt with the same letters. Command–Shift–R browses a random album. In Settings → About, tap or click the logo four times to read the last six days of activity.
