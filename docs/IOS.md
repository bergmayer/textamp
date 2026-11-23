# iOS

The iOS target uses SwiftUI for native navigation, lists, gestures, sheets and
forms. The pinned GPUI Kit 0.9.0 / gpui-pre 0.3.11 dependency graph has desktop
platform backends and no UIKit application/window backend. It cannot host an iOS
application without a platform port. macOS continues to use GPUI Kit, and the
terminal continues to use Ratatui.

## Runtime and presentation

`ios/core` builds a Rust static library. Its C interface owns one
`app::runtime::Session` on a Tokio worker. `src/mobile` projects that session and
translates native gestures into shared actions. Playback, queue edits, navigation,
source routing, caches, downloads, search, Journey and request cancellation use
the same implementation as the desktop. Projection tokens retain their library
generation, and selection commands validate the owning column and item.

Swift calls the bridge on a serial worker queue. It receives coalesced view
snapshots separately from playback-clock updates; it does not perform network or
disk work on the main actor. Artwork bytes come from the shared caches. Character
art uses the same half-block and Braille encoders as the terminal presentation.
Named themes use the shared semantic palette for content and native navigation titles. Enabled text maintains at least 4.5:1 contrast on its reading surfaces; System uses iOS semantic colors, with secondary text opacity adjusted when necessary for legibility. Artist details offer Radio and browser search controls; Search settings choose Google or DuckDuckGo. Browser search opens the system browser.
SwiftUI observes the properties each view reads, so playback-clock updates do not
invalidate library navigation, Settings or toolbar menus.

In wide iPad windows, Library opens in the leading sidebar and Play Queue on the right. The sidebar button hides or shows Library while preserving the current columns.
The lists and details between them form a horizontally scrollable column chain.
Selecting a row reveals the next column. Tap a list title to filter, use its
header controls for playback and View Options, or close a column and its
descendants with the × button. The bottom queue button hides or shows the queue
beside Library or Now Playing. In the docked queue, choose Queue Tools → Edit Queue
to reorder or remove tracks. Done exits editing. Now Playing places artwork beside
the transport and waveform when both fit. Narrow panes and accessibility text
sizes stack them vertically.

On iPhone and in narrow iPad windows, the Library navigation stack displays one column at a time. Both layouts use the same explicit navigation path. Source changes reset the path, and logical destinations replace the corresponding entries. Swipe back and the
Back chevron return through its column chain. Touch and hold the chevron for
Up One Level or Return to Previous Location; return is unavailable without saved
navigation history. Root categories use icons and
disclosure arrows: tapping an arrow expands its children in place, while tapping
the category name opens those destinations in a column. Groups with 1–512 items, including Playlists, have an arrow to expand or collapse them inline. Tapping a column title
replaces it with its filter field. Swipe down in a list or form to dismiss the onscreen keyboard. Search queries, filters and unfinished form entries stay in place. Search opens a separate library-wide sheet.
View Options contains sort, grouping and artwork choices. The command palette
is directly available in the bottom bar and Settings. Search keeps its own focused toolbar.
Settings is full screen. Its About section uses the Textamp block-letter logo in the active theme’s accent color.
Settings uses SwiftUI forms, switches for independent options, navigation-link
pickers for themes, artwork, web search and streaming quality, and a stepper for
the Singles track limit. Streaming quality selects an explicit bitrate through
the shared settings reducer; changing it takes effect on the next track.
The Play Queue antenna menu lists queue types first, then Repeat and Continue with Related Tracks in its bottom Playback section. The bottom bar provides Play Queue, Search and the command palette. Available commands recently chosen from the palette appear first for ten subsequent openings, including across launches. They also remain in their normal menu groups. Available selection commands appear directly before the menu groups. Music searches also appear under Discover. Compact layouts also include Library to return to the root. Tapping the mini-player opens Now Playing in the wide iPad workspace. In compact windows, tap the transport bar to open Now Playing, or drag it upward. The panel follows the drag and opens fully when you release it after a swipe. A downward swipe from the header or center, a tap on the handle, or the close button returns it to the bar. A short, slow drag settles back to full height. Lyrics supports both large and medium sheet heights.
Failed Navidrome library loads retain existing cached data and offer Retry in the status area. The recovery notice stays visible until retried or dismissed. The command palette omits status notices so they cannot obscure commands; closing it restores the notice.

The mini-player keeps artwork, title, artist and transport controls in a separate
rounded surface. Seeking and time indicators belong to the expanded player.

Palette destinations use the mobile navigation adapter: Play Queue reveals its
column or selects its tab in compact layouts, while View Options, library switching and radio selection use native sheets.
Similar music and related artists appear in the detail panes. In an editable server playlist, tap Edit or use a two-finger selection gesture. Use the trash button in the column header to remove checked entries from the playlist. The Selected Tracks menu moves them up or down. Tap Done to return to browsing. Sheet identity follows the shared modal
state. Closing the palette restores the underlying form and its draft; delayed dismissal callbacks
cannot close a different sheet. Desktop-only commands are omitted from the mobile
palette. Palette music searches use the selected row or focused details, including similar
tracks, then the surrounding artist or album context, then playback. Search commands are
disabled without a music subject. Apple Music and YouTube are enabled by default;
Spotify is optional in Search settings. Apple Music sends the original query directly to the Music app, including punctuation and queries with no catalog match. Use ⌘⇧A on a hardware keyboard for Search Apple Music. Textamp opens the Apple Music search URL as an app-only universal link. iOS reports whether the link opened; Music controls the resulting screen. Spotify and YouTube prefer their installed app and use web results if no app accepts the handoff.
Long-press the Home Screen icon and choose Search to open library search. Matching playlists appear before tracks and open in the library.
Available recent and selection-specific commands precede the shared menu groups. Tapping
a group opens its commands and submenus; the navigation-bar Back button returns
to its parent. Typing searches all commands from any group. External keyboards
can use Up/Down to select, Right/Left to enter or leave groups, and Return to run
the selected command.

Catalog columns use a native UITableView with momentum scrolling, interactive
scroll indicators. Lists longer than 270 items show an alphabet index to the right
of the scroll indicator. Each letter jumps to its first occurrence without changing
the list's sorting. Scrubbing the index displays the selected letter at a larger size.
Its full row count determines scrolling geometry, while Rust projects at most 300
rows per column around the viewport. Visible artwork requests are batched, and
playback-clock updates do not replace row data. Row heights scale with Dynamic
Type; custom navigation and queue animations honor Reduce Motion. Background
presentation polling is reduced while the shared playback runtime remains active.
Image decoding and character-art preparation use a separate worker pool limited
to two concurrent operations, leaving command delivery independent of image work.
The native image cache is limited to 40 MiB of decoded pixels. The Navidrome
Library shows its root destinations together when the saved browsing catalog opens.
On a first import, browsing starts as soon as that catalog arrives. The remaining
tracks can continue loading in the background. Library scans remain cancellable
background work. Settings exposes per-library metadata, analysis and artwork cache
sizes, refresh and clear actions, as well as Show / Hide controls and Singles preferences under Library. Status updates appear above the mini-player or sheet footer
without replacing navigation or blocking interaction. Connectivity notices expire;
only offline state retains an indicator at the bottom.

A fresh cached catalog opens without waiting for the network. A completed
network catalog refresh starts a background fill of playlist contents and
album covers. Settings → Caches → Re-scan library now also refreshes them.
iOS, desktop and terminal use the same cache jobs.

Large Navidrome imports fetch up to four pages concurrently within each paginated
collection, retain server offset order and reject repeated items or failed pages.
Each request has a 30-second deadline and interrupted read-only requests retry
once. Imports have a finite page limit rather than a whole-library deadline, so
continued progress through a large library does not produce a premature timeout.

Queue rows support long-press dragging, Edit-mode reordering, swipe deletion and
contextual actions. A separate Queue Tools menu includes Save Queue as Playlist where supported
by the source. Now Playing exposes artist, album and track navigation, seeking,
volume and lyrics. Playback actions use icons with accessible labels. Column
headers hold compact playback controls when they fit, with a second row at narrow
widths. Album filters match both album titles and artist names. Visualizers
are not part of the mobile presentation.

Long-press menus download playlists, albums and tracks through the shared download
service. Track and album details also expose Download alongside their playback icons. Journey endpoints
open a search sheet with artist and album drill-down; selecting a track commits
the endpoint, while dismissing the picker preserves the selected track.

Named themes select a matching alternate app icon. System includes light and dark
icon variants. Assets are generated from the shared palettes with
`cargo run --example app_icons`; iOS owns icon-change confirmation.

## Build

Use Xcode, XcodeGen and the Rust iOS target for the selected destination:

```sh
rustup target add aarch64-apple-ios-sim aarch64-apple-ios
ios/build.sh simulator
ios/build.sh device Release
```

The scripts reuse the workspace `target` directory and generate
`ios/Textamp.xcodeproj` from `ios/project.yml`. Open that project and choose a
development team to install on a physical device. Command-line development
builds disable signing. Simulator builds target Apple Silicon; the deployment
target is iOS 17. Debug links the development Rust library; Release links the optimized
Rust library. Use Release when installing or measuring device performance.

For isolated device validation, launch with `--test-profile=NAME` (letters, numbers
and hyphens). This uses separate Config, Data, State and Cache directories beneath
Validation/NAME in the app container. Populate these with fixture data or a private
copy of the library before testing; normal launches keep the regular profile.
Validation sessions keep the screen awake while the app is foregrounded, avoiding
lock-related suspension during measurements. A normal launch removes the app-owned
Validation folders from Application Support and Caches while retaining the normal
profile. Use this cleanup rather than device-container replacement commands.
For a physical device, bind the fixture with `--host MAC_LAN_ADDRESS` and seed its
profile with that address; loopback addresses only work in the simulator. The
fixture's `--fail-catalog` option exercises the recoverable library-error UI.

Native interaction tests use `--silent` (without a hardware audio device) and the dedicated
loopback fixture. The native-playback cache responsiveness case explicitly uses
the audio backend; set the isolated simulator profile's volume to zero and run
the fixture with `--scan-delay 90` for that case.

```sh
python3 tests/fixtures/desktop_ui_server.py --catalog --sonic --port 48791 --ready-file "$(mktemp)"
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  xcodebuild -project ios/Textamp.xcodeproj -scheme Textamp \
  -destination 'platform=iOS Simulator,id=SIMULATOR_UUID' \
  -derivedDataPath target/ios-xcode CODE_SIGNING_ALLOWED=NO \
  -only-testing:TextampUITests/LibraryFlowTests test
```

## Storage and platform boundaries

Configuration, credentials, state and downloads live in the application's
Application Support sandbox; evictable caches live in Library/Caches. These paths
are passed through XDG overrides before opening the shared runtime. There is no
automatic transfer of the Mac profile or credentials to the phone.

Navidrome and WebDAV connections can be added on the phone. Desktop filesystem
paths are not portable to the iOS sandbox; Files document-provider import and
security-scoped folder bookmarks are not implemented. Audio uses the shared
backend with an iOS playback audio session, remote transport commands and
interruption handling. Now Playing publishes the original cached artwork independently
of the in-app artwork rendering preference, along with title, artist, album, position,
duration and playback rate. iOS owns the Lock Screen artwork expansion gesture and
presentation. Play and Pause remote commands are idempotent; headset removal pauses
playback, and eligible interruptions resume only when the system allows it. Starting a track activates the playback session even if the app already has a mixer. Following an interruption or media-services reset, playback reopens the output and resumes from the saved position.
Hardware output and background playback require physical
device verification.

Downloads can group tracks by album from View Options. Opening a downloaded
album shows its local tracks and pending transfers. Offline album details use
saved metadata and cover art where available. Offline artist details list the
artist’s downloaded albums, with their saved track counts. The persistent
Offline indicator appears when iOS has no network path or Offline Mode is enabled.

Completed downloads are durable. Transfers run while the app is active; they
are not delegated to an iOS background URLSession, and pending jobs are not
restored after process termination. Network reachability updates offline state.
Downloaded metadata and audio remain available; network-only discovery reports
“Unavailable offline”.

Long browser lists expose a persistent scroll thumb. Tap the scroll track to jump or drag the thumb to scrub. Alphabetically sorted lists also have a letter index. Drag two fingers across library rows to enter selection mode. Tap rows or drag again to change the selection, then tap Done. In an editable playlist the gesture opens playlist edit mode. Column titles stay on one line and scroll after ten seconds when truncated, unless Reduce Motion is enabled.

Library uses a large leading navigation title. In compact layouts its bottom-bar button returns to the root. Back moves up one hierarchy level; holding Back offers return to the prior explicit library location, also exposed by the adjacent return arrow while history exists. Library-location history retains six entries. Settings and Now Playing share the main bottom bar. Detail playback controls sit beneath artwork; holding Play offers Play and Play Next. Following-track operations are contextual menu commands. Now Playing uses one position scrubber in its main transport area, with a compact handle over the shared waveform and time labels underneath. Before samples are available, the same control displays a normal progress track; waveform arrival preserves the control, geometry and active drag. Its expand control opens a landscape waveform with a Done button; closing it restores the prior orientation. Waveform and transport seeking share the playback engine.

Cache settings show shared background work, playlist track-list completion counts and percentage, artwork checks, completion, and errors. Playlist metadata proceeds independently of bounded artwork downloads.

Album collection destinations keep per-library disk snapshots and preload with
bounded concurrency. Opening a cached collection displays its list before the
server refresh completes. Large columns project a bounded visible row window;
the complete cached list remains available for filtering and alphabet navigation.
Ordinary notices dismiss automatically after four seconds, errors after eight,
and VoiceOver announcements after twelve. Active progress remains visible.
Play Random Albums starts continuous album playback. Go to Random Album opens an album for browsing without starting playback.


Appearance uses a native Theme pop-up menu. Choosing a theme updates the current Settings page without leaving it. iOS icon colors follow the theme's accent. Now Playing keeps its volume row outside the scrolling artwork and transport area. Its Lyrics button opens lyrics for the current track. Follow Playback scrolls to lines as their timestamps pass and highlights words when word timing is available. The switch is disabled when the selected language has plain text lyrics. Switching follow off permits free reading; switching language reuses the loaded response. Track changes replace and cancel the prior request. Empty responses show No lyrics found; network errors offer Reload. An explicit web search is available when lyrics are missing. Requests use the existing Navidrome account and authenticated enhanced getLyricsBySongId form endpoint. The app does not persist lyrics or write music files.

The Home Screen quick action Play Random Albums works through the shared radio command. It waits for the selected source to be available and runs once. It never contributes to command-palette recency.
