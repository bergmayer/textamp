# macOS desktop frontend

The desktop-enabled `textamp` executable uses GPUI with GPUI Kit 0.9.0 for
Graphical mode and the existing Ratatui frontend inside libghostty-vt for Text
mode. `--terminal` runs that TUI in the invoking shell. The default non-desktop
Cargo build remains the standalone TUI.

## Ownership

`desktop::engine` runs application dispatch on a Tokio worker thread. GPUI sends
typed commands and renders presentation snapshots; it does not perform provider
requests. Commands that alter state execute in order. Input activation resolves
the current selection in the worker, preventing a presentation snapshot from
playing an outdated selection.

`desktop::browser`, `catalog` and `player` project shared state into browser,
detail and playback models. Immutable browser rows and track metadata are shared
across snapshots. Queue and column projections refresh on state-changing work;
passive playback ticks reuse them. `connection` owns Settings submission status by
window identity. Credentials stay on the command path, outside snapshots.
`surface` projects workflow content, `workflows` owns persistent workflow sessions
and their provider task leases, and `surface_view` renders Kit dialogs or native
windows according to the application window mode.

Kit supplies standard controls, forms, description lists, menus, popovers,
breadcrumbs, dialogs, virtual lists, scrollbars, sliders and resizers. Custom
layout composes Miller columns, artwork, queue tools and audio visualizers.

Library and Now Playing share one window by default. View → Modes → GUI: Multiple Windows enables persistent independent windows for Now Playing, the visualizer, the
command palette and workflows. The visualizer retains its shared tab and playback
data in its own resizable window, available through Window → Visualizer; Single Window and Text keep it embedded. View → Modes → GUI: Single Window returns to one document window
with temporary Kit dialogs. Settings uses a Kit overlay in single-window mode and a native window in multi-window mode. New native windows cascade within the usable display bounds. Appearance includes a shared Mode control for Text, GUI Single Window, and GUI Multiple Windows. The choice persists. Closing the main window in single
mode quits the application. Multiple-window mode keeps playback and menus alive
when every window is closed; an application-level coordinator opens requested
windows independently of any document view's lifetime. ⌘1 and ⌘3 show Library and
Now Playing. The Mac
GUI uses the macOS system output without device-selection or pairing controls.
View Options stays a fixed size across all column and grouping states.

## Interaction

**Audio Player** is a main-sidebar destination. The Window menu opens the same
column within the library window. Choose a mounted volume and a playlist, then
use **Sync Now**. The column also includes reconnect sync, cancellation, eject,
and progress. See [MP3 players](MP3_PLAYERS.md) for ownership and firmware
limitations. Manual sync opens a review with expandable file groups and an explicit Apply Sync button. On this player is a read-only, searchable list grouped by library. Both views use Kit disclosure controls and virtualized file lists.

The sidebar groups albums, artists, tracks, playlists and optional radio stations.
Album collections have a disclosure control. Playlists reside in the sidebar.
Settings controls sidebar entries and collapsed sections. Reset View restores
panel sizes. Detail metadata links artists, albums and genre tags to the library
and reveals the selected row in each ancestor column.
Clicking playlist tracks gives that column keyboard focus; arrows change
selection, Enter opens the selected item like Right Arrow, and Command-Enter plays it and following tracks.

Clicking selects or opens adjacent details without playback or recentering the
clicked column. A child opens while focus remains in its parent. Arrow keys move
through the hierarchy. Enter opens the selected item like Right Arrow; Command-Enter plays a track
and the following tracks. Double-click plays. Context menus target the clicked
item and preserve an existing multi-selection when applicable.

Alphabetical columns expose a fitted alphabet rail alongside a scrollbar that stays visible while the main list overflows. Alphabet jumps center the selected row, clamped at the list ends. Reverse alphabetical order reverses the rail.
Letter keys jump by initial; Shift-letter uses the second letter within the
current initial. A Kit text field above the list provides filtering and is
shown by clicking the column title or by focusing the filter. Filtering applies
only to its owning column; child columns remain navigable.

Columns and panels resize with dividers. View → Reset View restores dimensions.
The title bar contains clickable navigation breadcrumbs. The transport stays outside scrolling content and shows artwork. In single-window
mode it expands upward over the retained library. Artwork grows into the resizable
header, playback controls move beneath it, and visualizers and queue occupy the
space below. The transport and queue keep the same resizable column layout in
both views, including during the transition, so the controls retain their
horizontal alignment after resizing. A small chevron collapses the player. Separate windows retain the
bottom transport and its track tile raises the other window. Now Playing combines square artwork, metadata,
visualizers and a reorderable queue. Below 720 points wide or 520 points tall,
Now Playing responds with square artwork and minimal playback/seek controls.
Growing the window restores the full layout. Play Queue opens independently beside
the library or player, or in its own window in multi-window mode. Queue tools sit
in their own area at the bottom of that pane, above the fixed footer; Radio opens its sidebar columns.
Left from the first content column focuses the sidebar. Up/Down navigates visible
rows, Enter/Right opens them, and Left collapses a section. Sidebar menus and
disclosures remain Kit components; their tracked scroll container exposes the
scroll handle that Kit Sidebar keeps private, so keyboard focus remains visible.
Settings → Sidebar mirrors the displayed Albums, Library, AudioMuse, Playlists
and Radio Stations groups, with checkboxes for hidden destinations as well.
The title bar contains clickable breadcrumbs and the command-palette button.
Application status, loading and errors use an opaque Kit StatusBar fixed below
the transport. Its height is reserved even when empty, so messages never shift
the workspace. The focused column’s displayed-row count appears only there.
Notification lifetime and priority remain owned by AppState. Queue-edit and music-search
progress follows the task lease, stays visible with a spinner, and ends on
completion, cancellation or failure. Sidebar scrolling
uses the same dedicated Kit scrollbar rail when content overflows as the other columns;
Kit hides the thumb when the content fits.
Keyboard selection and scrolling happen immediately. Child inspection is delayed
250 ms during rapid movement, with source and selection identities checked before
dispatch and superseded listing tasks cancelled. Dependent columns, details and
artwork retain their prior presentation during the pause and the owned listing
request, then update together. Sidebar disclosure rows use the same Kit menu-item
geometry as top-level destinations; expandable groups have a trailing disclosure button and
category children have an additional 24-point indent.
Album View Options offers Large album covers with matching virtual row heights.
Album track columns begin with an Album Info entry. Arrow navigation and mouse
selection open its details pane without selecting or playing a track. Selecting
an album in its parent column opens the track listing without an automatic
album-details pane. Meters keeps its blue RMS-history chart below the stereo
bars, with a separate scale and space above the correlation indicator.

Settings is available from the application menu and Command-comma in an overlay in single-window mode and a native window in multi-window mode. Saving credentials does not activate a library or alter
playback. Other reading and configuration workflows use the same Kit theme and follow the
same presentation mode. Persistent workflow sessions keep their input, selection
and results when another workflow is opened. Provider completions retain their
request identity and task lease and return to the owning workflow. Closing a
workflow cancels its tasks. Readers from a previous library remain visible with
inactive controls until closed. The command palette opens from the Command-symbol toolbar
button, keyboard shortcut or colon outside a text field; Escape dismisses it
immediately, including when a query is present. The unfiltered palette shows available recent commands, selection-specific commands, and the shared menu groups. Settings, Switch Library, and Refresh Library appear in Textamp. Right-click menus retain the same selected-item actions.

The palette follows [Zed's command palette](https://github.com/zed-industries/zed/blob/main/crates/command_palette/src/command_palette.rs)
and [picker layout](https://github.com/zed-industries/zed/blob/main/crates/picker/src/render.rs):
a 608-point floating picker without a redundant title, search first, inset result
rows, fuzzy character highlights, trailing Mac shortcut hints and a Run/Return
footer. Only commands executed from the palette enter recent history. Promotion lasts ten subsequent palette openings and persists across launches; query edits and submenu navigation do not age it. Recent commands remain available in their normal groups. Kit owns input,
selection, keyboard navigation, virtualization and scrollbars. Kit's built-in
filter only matches substrings, so the application supplies fuzzy-ranked items
through its unfiltered command API. Fuzzy match ranges use styled text inside
Kit's interactive rows because Kit Label's match highlighting accepts contiguous
strings. The temporary picker sizes to its results. It uses the existing fuzzy matcher rather
than importing Zed's workspace-specific picker. Textamp has no keymap editor, so
it does not present Zed's Add/Change Keybinding action. In multiple-window mode,
the palette remains open after execution; Escape or closing its window dismisses
it. Temporary dialogs retain Kit's focus restoration and dismissal ownership.

## Runtime and verification

A library generation rejects work from a different source. Each replaceable
provider operation also carries a request identity and task lease. Closing a
biography reader drops its request lease. Its lease is its loading state.
Radio preparation and queue edits retain provider task leases. Source
activation is distinct from account edits and cache maintenance.

Cached startup precedes network refresh. Ordered playlist snapshots avoid repeated
getPlaylist requests when browsing. Background cache maintenance fills every
unique playlist album cover with bounded concurrency, independently of visible
artwork rows; clearing artwork cancels that work and fences pending writes. Artwork, metadata and AudioMuse caches
retain library scope. Passive requests while disconnected wait for activation;
explicit operations explain whether a library is connecting or must be selected.

Build with `cargo build --release --features desktop --bin textamp`.
`packaging/macos-desktop/build.sh --preview` packages the incremental development
build as a separate Preview app with isolated configuration and caches. It reuses
the `desktop-tests` feature graph; add `--release` for an optimized Preview. `--no-audio` disables native audio output.
Automated interactions use `tests/fixtures/desktop_ui_server.py`, synthetic
credentials and isolated XDG directories. Personal servers are not mutation-test
fixtures. Run the affected frontend test suites, formatting and strict Clippy as
specified in AGENTS.md. Optimized packaging is reserved for release requests or
performance verification.

`cargo test --features desktop-tests --lib desktop::frame_tests` runs GPUI frame
and mouse-input regressions without native audio or provider access. The optional
feature enables the pinned GPUI runtime's test support. These tests check sidebar
hit targets, loaded track and playlist rows, album title fit and width caps,
divider dragging, and a library viewport that fills
its allocated space in both window modes. Run with isolated XDG directories.
The ignored actor test `actor_sidebar_destinations_load_during_slow_similarity_requests`
uses the loopback catalog fixture with `--sonic --sonic-delay 3`, an isolated
fixture account configuration, and `TEXTAMP_ACTOR_FIXTURE_URL` set to its loopback
URL. It verifies sidebar destinations remain responsive during background work.

## macOS design conformance

The GUI's visual target is the standard SwiftUI experience on the supported
macOS release, using GPUI Kit 0.9.0 controls where they can satisfy that target.
Using Kit does not make an interface native by itself. The application supports
macOS 15 and later; materials and control geometry need appropriate availability
handling rather than a single painted approximation of a particular OS release.
The reviewed host runs macOS 27.0.1.

### Shared semantics and platform boundaries

Settings categories, preference identities and validation, detail sections,
playback modifiers, queue controls and navigation history are shared application
models. Kit Settings reports category selection to the adapter; the TUI uses the
same category and draft state in its popover. Native layout and terminal geometry
remain independent. TUI playback controls belong in detail content; GUI headers
may expose the same commands. The contextual TUI palette replaces GUI context
menus.

Named themes use one semantic palette. System appearance in the embedded terminal
uses the current native Kit colors; the external terminal retains terminal-owned
default colors. Native Settings and platform dialogs retain macOS conventions.
A complete VoiceOver, text scaling and all supported macOS appearance-matrix
acceptance pass remains separate from compilation and the interaction checks
recorded below.

### Window and control specifications

Control selection follows Apple's distinction between
[pop-up pickers](https://developer.apple.com/design/human-interface-guidelines/pop-up-buttons)
and [command pull-down menus](https://developer.apple.com/design/human-interface-guidelines/pull-down-buttons).
Window roles follow the [window guidance](https://developer.apple.com/design/human-interface-guidelines/windows);
Settings retains its independent [Mac settings presentation](https://developer.apple.com/design/human-interface-guidelines/settings#macOS).

| Surface | Presentation and controls | Layout and behavior |
|---|---|---|
| Library | Native primary window; Kit toolbar groups, sidebar menus/disclosures, ListItem rows, resizers, description lists and scrollbars. | Leading navigation, central Miller columns, trailing selected-item inspector. Selection is distinct from playback. Inspector actions stay with their item; column headers hold column commands. |
| Now Playing | Same primary window in single-window mode; independent native player in multiple-window mode. Kit buttons, sliders, mode selector and queue controls. | Square artwork and track information above visualizers; queue and its tools stay together. Keep the transport outside scrolling content. Visualization choices use horizontally scrollable, content-sized segmented tabs without a duplicate pop-up selector. Distinguish expanding a visualizer from native Enter Full Screen. |
| Mini-player | Responsive presentation of Now Playing, retaining native chrome. | Square cover, title/artist and minimal playback/seek controls. Growing restores the full layout and queue position. Every icon needs an accessible action name and the seek slider needs an accessible time value. |
| Settings | Temporary Kit dialog or independent native window according to mode, with grouped Kit settings pages and Form/Field content. | Retain the category sidebar for settings categories and account/cache organization; this avoids a crowded toolbar. It must use Mac typography, appropriate pane titles, a single pane-selection owner and predictable focus. General preferences save immediately. |
| Libraries and connection editor | One account row per Navidrome connection; folders are scopes, not separate accounts. Masked credential Inputs, Form/Field, one Save Connection action and Cancel. | Show included folder names. Editing or saving never switches the active source or playback. Use a native folder chooser with a displayed path for local libraries; typed paths may remain as an additional keyboard option. Place advanced analysis fields in a disclosure group. |
| Artwork, Audio, Sidebar and Search settings | Radio groups/pop-up pickers for one-of-many settings; checkboxes for independent flags. | Show current values without requiring repeated clicks. Keep rendering controls with artwork, streaming quality with audio and discovery-service flags with search. Audio routing remains macOS-owned. |
| Cache & Scanning | Grouped per-library metadata/artwork/analysis information, standard progress and command buttons. | Separate Scan/Refresh, Cancel Scan and Clear operations; show task state beside the relevant scope. Use determinate progress only when a real total exists. Destructive labels and confirmations identify exactly which cache is affected. |
| About | Reusable branding content in Settings About; expose it from the app menu through a standard About presentation. | Original ANSI logo, Textamp name and version, then concise product information. Treat the ANSI work as an accessible image; its monospaced decorative rendering is separate from UI typography. |
| View Options | Fixed Kit dialog or native utility window according to mode; labeled sort radio group, direction control, independent checkboxes. | Keep one stable footprint. Date Added is available for Recently Added and restores the provider's original newest-first ordering; Oldest First reverses it. Covers, large covers and playlist grouping are independent options where applicable. Changes are immediate; no Apply/Save footer is needed. |
| Library chooser | A current-selection picker or, for many sources, a compact native list window. | Differentiate All Folders from a named Folder scope. Click and keyboard activation choose the same scope. The account-wide catalog combines both Navidrome folders; it does not require activating two mutually exclusive folder views. |
| Search | Temporary dialog or persistent native result browser according to mode, Kit search field/scope selector, ListItem results and an adjacent inspector. | Keep the query and scope visible, stable window dimensions, and explicit loading, empty and failure states. Selection opens information; playback remains an explicit action. |
| Artist/Album/Track Info | Selected-item inspector using borderless DescriptionList, artwork and relevant controls. Optional independent reading windows remain native. | Album Info is an explicit entry in album track columns. Link metadata to the library, with visible keyboard focus as well as hover affordances. Use aligned label/value columns, square artwork and adequate separation. |
| Biography, lyrics, text readers and Help | Temporary dialogs or resizable persistent native readers; selectable Kit TextView and structured help content. | Use content titles, readable line lengths, normal copy/select behavior, a scrollbar when content overflows and platform close controls. Keep source/external links separate from playback controls. |
| Artist Mix | The Radio detail pane with a search Input and checkbox artist list, limited to 12 selected artists. | Show selected artists and a distinct Start Radio action. Maintain Cancel and keyboard selection. |
| Journey / describe-music generation | Destinations under AI in the sidebar with inline forms, result browsers and explicit actions. | Put start/end selection beside their values, use a number input for track count and a search field for lookup. Preview Journey builds a route; Play Journey explicitly replaces the queue. Swap Tracks is secondary and Back is navigation. Show task cancellation and errors in context. |
| Save/rename prompts and destructive confirmations | A native task-attached sheet/alert where supported, or a native utility window with equivalent semantics when required. | Use Kit-managed temporary dialogs in single-window mode and task-attached native confirmation surfaces in multiple-window mode. Name fields have visible labels. Save/Cancel and destructive roles are explicit; destructive operations do not become the default solely because their button is last. Other windows remain usable when the task permits. |
| Command palette | Kit Command inside a managed Kit Dialog in single-window mode or a persistent native window in multiple-window mode. | Retain the Command-symbol toolbar button and colon shortcut outside inputs. Escape immediately dismisses it, including a nonempty query. Keep focus restoration and domain commands distinct from generic utility-window cancellation. |
| Radio and queue edits | The Radio sidebar destination uses compact Miller columns for continuous stations and subcategories, including Artist Mix; icon buttons apply queue edits once. | The Radio icon uses the active theme accent while a station is engaged. Allow Same Artist is an Audio settings checkbox. Shuffle/repeat are stateful icon controls with names, tooltips and accessible values. |

### Typography, spacing and depth

The system UI family is already selected through `.SystemUIFont`; keep it. Apple’s
[typography specifications](https://developer.apple.com/design/human-interface-guidelines/typography#Specifications)
list macOS Body as 13/16 points, Callout 12/15, Subheadline 11/14, Title 2
17/22 and Title 1 22/26. Use these as semantic roles with the documented emphasis
weights, rather than adding independent font sizes in every renderer. Reserve
10/13 captions for secondary instrument labels. ANSI glyph sizing is artwork,
not a small-text precedent for the interface. macOS does not support iOS Dynamic
Type; test the actual Mac accessibility and font behavior instead of promising an
unsupported automatic adaptation.

Use shared spacing tokens: 4 points for tightly related details, 8 between related
controls, 12 between subgroups, 20 at ordinary form/content edges and 24 between
major sections. These are project starting values, not universal HIG mandates.
Intrinsic system-control geometry, localization and supported macOS metrics take
precedence. Align form labels consistently within each form, use one control
size per row, and keep text/list content denser than artwork-heavy rows.

Create depth through sidebar/content/inspector roles, dividers, selected states
and native window/material behavior. Do not add shadows to every panel or fake
translucency with gradients. Use system materials where the framework can host
them correctly; an opaque semantic fallback is preferable to an inaccurate glass
simulation. Honor Reduce Transparency, Increase Contrast and inactive-window
appearance. Keep artwork and visualizer color independent from control tint.

### Deliberate exceptions

| Exception | Reason and required safeguard |
|---|---|
| Miller columns instead of a standard navigation stack | Large music hierarchies benefit from simultaneous parent/child context. Preserve stable parent selection, proportional resizing and keyboard traversal. |
| Persistent overflow scrollbars | Scrollbars remain visible while content overflows for discoverable scrolling in large catalogs. They disappear when content fits; the filter field sits outside the scrolling list. The alphabet rail does not replace the main scrollbar. |
| Alphabet rail and second-letter jumps | Direct navigation is useful for catalogs with tens of thousands of artists. Show it only for alphabetical ordering; reverse it with that order and prevent overlap with text. |
| Toggleable column-filter field | Clicking the title replaces it with a stock Kit input in the header. It is hidden until requested to preserve browsing space. |
| Return opens; double-click plays | Return shares Right Arrow navigation. Double-click plays albums, playlists and tracks; Command-Return plays a track and those following. Queue Return starts at the existing queue position. |
| Persistent playback controls | Playback needs stable access across music tasks. The collapsed transport sits below Library; in the expanded single-window player it sits below the artwork header. Controls remain outside scrolling content with equivalent menu and keyboard commands. |
| Settings category sidebar | Eight categories plus provider/account/cache grouping benefit from a stable sidebar instead of a crowded toolbar. This does not justify non-Mac controls, title behavior or competing focus state. |
| Command palette and Command-symbol toolbar button | Power-user discovery complements the native menus. It cannot be the only GUI for settings, radio controls or ordinary task commands. |
| Music-specific artwork, meters and visualizers | No stock Kit widget represents these domain displays. Keep them custom, with truthful scales, accessible descriptions and responsive layout. |
| ANSI About artwork and the Terminal Note app icon | They identify the application. Retain standard Mac text and controls around them; expose an accessible logo description. |
| Unfilled native-material or SF Symbol capability | A specific renderer/platform limitation may justify a semantic opaque surface or a matching Kit icon. Document the affected feature, and use a small native bridge when it materially restores expected behavior. It is not a blanket exemption for generic web styling. |

### Native acceptance

Compare against a small stock SwiftUI reference on the supported macOS versions:
light/dark appearance, non-blue system accent, Increase Contrast, Reduce
Transparency, Reduce Motion, active/inactive windows, standard control sizes,
text scaling and long/localized labels. Check keyboard-only and VoiceOver flows
for every window role above, including menus, editable fields and cancellation.
Check short/wide and tall/narrow layouts, retained geometry and mini/full-player
transitions. Use isolated XDG directories, loopback fixtures and disabled audio.
Compilation and component inspection are not visual acceptance.

Primary references: [macOS](https://developer.apple.com/design/human-interface-guidelines/designing-for-macos),
[windows](https://developer.apple.com/design/human-interface-guidelines/windows),
[toolbars](https://developer.apple.com/design/human-interface-guidelines/toolbars),
[settings](https://developer.apple.com/design/human-interface-guidelines/settings),
[typography](https://developer.apple.com/design/human-interface-guidelines/typography),
[pop-up buttons](https://developer.apple.com/design/human-interface-guidelines/pop-up-buttons),
[pull-down buttons](https://developer.apple.com/design/human-interface-guidelines/pull-down-buttons),
[selection and input](https://developer.apple.com/design/human-interface-guidelines/selection-and-input),
[accessibility](https://developer.apple.com/design/human-interface-guidelines/accessibility),
[materials](https://developer.apple.com/design/human-interface-guidelines/materials).

Journey presents endpoint choices using Kit Form/Field, labeled finder inputs
and separate Preview Journey and Play Journey actions. Back to Results is a secondary
finder action; it never receives default-button styling. Every returned path
reserves both endpoint artists and removes repeated intermediate artists while
preserving path order. Missing artist metadata or an insufficient distinct-artist
path fails without replacing playback. The GUI never displays the terminal Queue
view as a library destination; playback queue controls live in Now Playing.

Sidebar keyboard playlist inspection uses the same 250 ms pause as Miller-column
inspection. Deferred commands retain playlist identity and library generation,
check that the row remains focused and visible, and preserve sidebar focus.
Right/Return enters the content column without reloading an already-open playlist.
Queue remix actions use compact Kit icon buttons beside shuffle and repeat: Twins,
Twofer, Stretch and Doppelganger. Each has an accessible name and a brief tooltip
with a 180 ms delay. Unavailable actions are disabled. The Radio button opens the
Radio sidebar destination, with the active station indicated by its accent color.
Allow same artist is a standard checkbox in Audio settings.

Miller panel groups contain visible columns and Details, excluding metadata-only
and hidden navigation columns. They divide space equally, with a 260-point automatic
minimum. Deeper hierarchies scroll horizontally using the Kit scrollbar; opening a
child reveals it without changing vertical selection. Kit dividers remain draggable,
and selection-only updates retain their widths. The layout signature uses stable
column identities, the inspector and container width rather than row contents.
View → Reset View restores equal proportions. The application mode remains a checked choice
in View, Settings → Appearance and the command palette.
Each Miller-column header includes a small Kit X icon that closes that column
and its descendants, preserving the parent and rejecting stale identities.
Native menus group sorting, view options, visualizers, audio and provider operations.
Sidebar category children use a 20-point indent; disclosures and top-level icons
remain aligned. ⌘[ invokes Close Column;
⌘W remains Close Window. Terminal bindings are independent.

Column transitions use Kit's bounds interpolation over 180 ms and relayout text
and hit targets at the presented bounds. Pointer interaction cancels the tween so
resize handles remain direct. The single-window player uses one reversible Kit
transition over 240 ms for its sheet, square artwork and shared transport. The
library fills its allocated viewport and remains mounted beneath it; there are
no duplicate playback sliders.
Kit handles Reduce Motion and transition retargeting. Custom geometry is limited
to pane composition and the shared artwork/transport morph; Kit owns the controls,
resize handles, focus and domain command dispatch.

The detail pane has a matching Kit close icon. Closing it preserves the browsing
columns and selection; selecting or navigating to another entry shows its details.

Journey uses a bounded Kit Input for maximum tracks. The pinned Kit
NumberInput assigns the same focus handle to its spin-button container and its
text field, which panics in GPUI accessibility prepaint when focused in a native
window. The normal Input retains numeric bounds without that duplicate focus
node; no custom stepper or framework fork is used.

Go to Random Album (Discover menu, ⌘⇧R) reveals an album from the active catalog without changing the queue or playback. The TUI exposes the same command with Ctrl+Shift+R. It requires a catalog source; folder libraries retain Play Random Albums. Title-bar breadcrumbs use selected destination names, and the Details pane shares the browser background and header styling.

Search uses Kit Input with a magnifying-glass prefix, built-in clear control, rounded semantic fill and a full-width query field. Journey uses one surface in either window mode, with both editable endpoints above its inline track search, maximum length, route preview and explicit Play Journey action.

Library jumps use the shared OpenInLibrary reducer, including related-track selection, random albums and search results. Track destinations select the canonical artist, album and track before opening details. Back restores the prior library view; the six-entry history records jumps, not ordinary browsing. Compilation branches and stale-completion guards apply to these destinations. Split view uses the embedded Now Playing pane without a second regular player window; explicit full-screen visualizer remains available. The macOS icon derives its palette from the active theme and updates the Dock and the writable app bundle’s Finder icon through AppKit, using the shared Terminal Note cell-grid artwork.

The Details pane participates in equal-width layout alongside all visible Miller columns. Its header identifies Album Details, Track Details or Artist Details. Single-window temporary dialogs dismiss on title-bar clicks. Held-arrow navigation accelerates after a short delay without issuing intermediate inspection requests. Queue remix icons sit beside shuffle and repeat; Radio station selection uses compact Miller columns in the Radio browser. Search input Return submits the omnibox, and type tabs filter its existing results.

Artist columns pin Compilations directly below All Tracks. All Tracks playback
uses the same small header play icon as playlists. Settings, Search and Command
Palette live at the left of the status bar below the panes. The palette icon
toggles its native window; Escape dismisses the single-window overlay. Each browsing column has a clickable title that opens its Kit filter input without truncating the hierarchy.

View Options uses a fixed compact 360 × 340 logical-pixel utility window, a concise
title, a subdued column-context label and closely spaced stock radio/checkbox
controls. Its single-window dialog uses the same content with one padding inset.

Journey phases are Choosing, Generating, Ready and Failed. Generation retains the
workflow, shows progress and supports cancellation. Results are reviewed before
replacing the queue; stale completions after cancellation are rejected. The TUI
provides the same preview and playback boundary. Radio uses an icon with a theme-accent active state and a full station browser. Album playback/enqueue and
artist radio controls occupy the typed Details header. Details takes keyboard
focus with Right or a click, returns focus with Left, scrolls with Up/Down and
closes with the standard Close Column command.

Continuous playback is expressed through stations rather than queue-insertion
modes. Artist/Decade/Sonic stations cover artist, era and evolving sonic streams;
Stay Close Mix retains
an explicit seed. The Artist Mix chooser is a single searchable Kit checkbox
list with selected-name tags and explicit Start/Cancel. Seed-based stations reuse
the track-finder form without Journey’s count/end-point controls.

The queue control row includes a matching Undo icon beside Clear Queue. It
restores the latest queue edit and is disabled when no queue snapshot exists;
the app-wide Undo command and shortcut remain available.

Playback and queue actions use stock Kit icon buttons in Details and collection
headers. Track actions provide an explicit Option-click alternative for the
selected track and following tracks; tooltips show that alternative while Option is held. The
two buttons are Play and Add to Queue (plus). Option-click Play supplies Play Next
without a duplicate button. Album and playlist actions operate on the complete collection.
Single-window palette hosting retains Kit's backdrop so outside presses dismiss
it without passing through to the underlying toolbar or reopening the palette.

Search uses Kit Input and the small segmented TabBar variant as a search scope
bar. The empty panel is compact, with a Kit Empty state; results expand its usable
area. Scope labels use title case. This follows Apple's search-field/scope-bar
pattern: https://developer.apple.com/design/human-interface-guidelines/search-fields

Settings keeps Kit Settings, SettingPage, SettingGroup and SettingField. Kit 0.9
adds sidebar children for titled groups, so section headings use read-only Kit
Label items inside untitled groups. The navigation remains flat while libraries,
layout, sidebar choices and cache sections retain headings in the content pane.

Manual Queue is available from the Radio browser, shared palette and Queue Type
menu. It retains queued tracks and prevents further station refills. Clear
Queue also clears the active station. The Queue header shows its current mode and a Kit pull-down for changing it;
radio navigation also remains in the sidebar, palette and native menu.

Play Queue and Now Playing are independent presentations. In single-window mode,
Queue rises beside the library and the Kit divider reallocates width to the browser;
Now Playing rises over the remaining content independently. Reduce Motion disables
both transitions. In multi-window mode Queue has its own native window and keyboard
focus, without a second transport. The queue list uses the shared playback occurrence
identity, including duplicate tracks. The queue button uses the standard list icon.
The utility/status footer remains fixed at 40 points. Queue Tools uses a Kit dropdown in the queue header beside Queue Type. A clipboard and pencil
identify queue editing and a broadcast tower identifies queue type/radio. Active
queue modes remain highlighted; tool toggles are checked inside the menu. The default Queue width is 280
points; its divider remains adjustable.
The transport uses left-aligned playback controls, a lighter track-information area
with a seek slider, then track actions, lyrics, queue and volume. Equal trailing
and leading reservations center the information area; when the Queue is open,
the leading reservation compensates for its width while room remains for labels. Muting preserves the
restore volume while showing zero on the slider and a crossed-out speaker.
Queue Type lists Manual Queue and stations first, followed by Repeat and Continue with Related Tracks at the bottom. Auto-Mix
is an independent switch with an overlapping-circle Kit icon. Queue edits remain one-time operations.
Auto-Mix uses equal-power overlaps up to four seconds, adjusted for incoming readiness
and outgoing playback time, for tracks at least twenty
seconds long; it does not perform tempo stretching or beat alignment. Native system
output supports overlapping sinks.
Lyrics uses the shared structured-lyrics state and active Navidrome authentication. The reading window offers Follow Playback, language tabs, Reload, and a Kit scrollbar. Lines and optional word cues follow the actual playback position. Local files show embedded text; empty results and request errors remain distinct. No music-file writes or additional persistent lyrics cache are performed.
Journey uses separate Kit search inputs for Start and End with results immediately
under the active field. Selected tracks retain their artist labels. The form is
compact until results need space. Header playback controls are ordered Play,
Add to Queue; tooltips reflect the active modifier operation.
Details artwork is a non-shrinking square within the scrolling metadata pane.
Similar-track selections inspect that track in place, using the shared related
track detail state and seed validation; album recommendations still open albums.
Navidrome's folder projection excludes artist directories without owned albums
and artists whose catalog consists only of compilation appearances.
Genres retains its saved AlbumGenres identifier. Decades groups cached albums by
release year in the shared core, with the same sidebar setting and navigation in
both frontends. Unknown dates do not create an empty decade. GUI About uses the
clean block-letter ANSI wordmark at a larger monospace size.

Artists precedes Albums in the desktop sidebar. The Albums section includes Compilations, using the shared compilation loader; the desktop artist root omits that duplicate destination. The terminal retains its compilation entry in its own navigation layout.

The Queue Tools dropdown includes Save queue as playlist, which opens the shared name dialog. It is disabled for an empty queue and preserves the existing menu and terminal save commands.

Playlist and queue export use native platform Save dialogs, with shared M3U8/CSV serialization and file writes in the app core. Submenu labels omit palette namespace prefixes while retaining their original command identities. Search-driven library jumps update desktop navigation focus and scroll positions through the same history signal as direct jumps. Playback controls retain their horizontal position while Now Playing expands, independently of the Queue panel.

The Now Playing information card uses a four-column Kit DescriptionList for
technical fields and paired artist/album fields and a full-width genre row, without a nested
scroll view. Its transport embeds seeking in the track-information area, with volume at the trailing edge.
Utility messages occupy the right edge in place of the item count. Queue tools
stay beneath the independently visible Play Queue. While a station is active, its Queue Type pull-down offers Manual Queue, related continuation and stations.
Stopping converts the radio queue into a regular queue without restarting audio,
and invalidates outstanding station refills.

Play Queue has a separate queue-button dismissal. Embedded Settings and workflows use Kit DialogClose with an XSmall Kit Button,
matching column close controls. Kit's built-in dialog close control fixes its size
to Small and exposes no size override; the composed close control retains Kit's
cancel dispatch and on-close behavior. Native workflow windows retain their
system-owned close control without an additional content close button.

DesktopFullscreenView records four exclusive open presentation states separately
from the TUI viewer: artwork, artwork with transport, visualizer, and visualizer
with transport. Arrow keys cycle them; Escape closes the viewer. Transport modes
reuse the normal playback controls and occurrence-guarded Up Next preview.

Ordinary waveforms use labeled time rulers in both frontends.

Radio is a library sidebar destination in both window modes. Its queue button and native menu command return to those same compact, resizable columns, rather than creating a workflow window. Artist Mix is a peer station entry that opens the shared artist chooser. Browsing remains separate from playback. The column content reuses Kit lists, resizers, scrollbars and status bar; only the Miller arrangement is custom.

Artist Mix and track-radio choosers are inline in Radio's detail pane. Journey
and Describe Music are AI sidebar destinations with workspace forms in both window
modes. Their state and request leases remain in the shared reducers; they are
excluded from native workflow-window ownership. Switching stations cancels the
old chooser, and reselecting it constructs an active form. Desktop ⌘F invokes the
desktop Search command directly, including from forms. Separate desktop Similar
and Related explorers are removed because results already live in detail panes;
the TUI retains its existing navigation and commands.

Inline discovery forms use stock Kit fields with bounded input widths; their result
lists retain the available pane width. Actions follow the inputs. Radio reserves a
footer only while station controls are available; application messages remain in
the shared utility bar. View Options shows its title and target once each. The
visualizer retains its Kit tabs, with small adjacent controls aligned in one row.

Describe Music keeps the history and preceding result columns visible while a
new search is pending; matching results replace the child listing. Its prompt
has a Search action and no separate History button. The history itself restores
saved results from the active library.

The visualizer selector uses content-sized, horizontally scrollable Kit segmented tabs without an overflow menu. The visualization area remains visible; its initial height is the minimum allowed by the Kit resize handle.
A continuous top separator frames the collapsed transport and Up Next together.

## Player layout rationale

The player uses an independent main content pane and Queue panel. Artwork
and aligned metadata share the upper content area. Transport text is centered
within its lighter field, with artwork to its left and playback controls preceding
the field. The visualization
uses the content background rather than another inset card. A single fixed
utility footer carries Back and app commands on the left. Queue Type and Queue Tools use stock Kit dropdowns in the queue header. Checked
items show active settings, and disabled items retain their capability rules.

This follows Apple's [layout](https://developer.apple.com/design/human-interface-guidelines/layout)
and [split-view](https://developer.apple.com/design/human-interface-guidelines/split-views)
guidance on alignment, hierarchy and grouping. The bottom utility strip is an
intentional departure from the general macOS recommendation to avoid critical
bottom-edge controls: it provides stable access while the player sheet moves,
and equivalent menu commands and keyboard shortcuts remain available. Native
window chrome and Kit controls are retained; the rendering framework does not
provide native Liquid Glass materials.

The default artwork/information region occupies about 70 percent of the available
player height, bounded by the content width; the visualization uses the lower
region. Metadata is a single label/value column, with scrolling in constrained
windows. Library jumps center selected rows in every visible ancestor column and
clamp at list ends. Pointer selection continues to preserve the viewport.


Meter scale labels use the actual canvas height and omit intermediate ticks when they would be less than 16 points apart. The minimum and maximum dBFS labels remain visible. Analysis downloads report byte progress, cancel when the owning track state is replaced, and have idle and total deadlines. Download URLs are removed from displayed request errors.

Development and test profiles optimize the audio codec and FFT dependencies while leaving the application incremental. Waveform and spectral analysis share the downloaded compressed bytes and run as independent CPU jobs. Each publishes its own result without waiting for the other; spectral failure cannot suppress a usable waveform.

Now Playing's action pull-down anchors above its ellipsis button. It omits commands
to restart the current track and opens Track, Album and Artist Details in the normal
library column path. These destinations share the core navigation intent and
request ownership with the terminal frontend. Back restores a library jump without
undoing queue edits; from Now Playing it returns to Library first.
All playlists share the Playlists destination and disclosure group. Playlist names
do not affect grouping, navigation, caching or available commands.

System appearance uses Kit's system-appearance synchronization with AppKit semantic
colors and accent. Window appearance and activation changes refresh those colors;
named palettes remain explicit alternatives. The System icon follows the system light or dark appearance. The terminal System palette uses terminal defaults.
Artist View Options includes a shared, saved Show compilation-only artists checkbox.
A hidden artist reached by an explicit name jump remains selected in the normal
artist column, with muted italic lettering rather than a disabled control. Turning
the option on makes these artists ordinary entries. Both frontends share this policy.
The queue reveal uses Kit's critically damped spring, retains the preferred panel
width during animation, and combines a short upward motion with a fade. Reduce Motion
still disables travel. During accelerated arrow navigation the focused row's visible
highlight is suppressed until key release; the actual selection remains available
to commands and accessibility.
The bottom toolbar has a pressed-state sidebar toggle and an inset item count.
Click a sidebar group name to open its destinations in a column. Groups with 1–512 items, including Playlists, have an arrow to expand or collapse them inline. Radio and nested destinations use columns.
View Options uses a fixed 540 by 300 point window with a two-column control grid.
Its controls also appear in View's Sort and View Options submenus with current
checkmarks and existing key equivalents. Large covers use 128-point artwork in
144-point rows. Empty waveforms have no time ruler; empty spectrograms have no
playback-position line.

Palette requests reopen a missing Kit dialog independently of the current music
screen. Dismissal validates the rendered surface instance, including its epoch,
so an old close event cannot dismiss a later palette. Shortcut hints occupy a
fixed trailing slot; command labels truncate within the available row width.

The pinned Kit Command component provides flat command items and custom row content,
but no nested command navigation. The palette retains that component for input,
list selection and activation, and projects shared menu paths into submenu rows
with disclosure icons and a Back item. Search matches the complete command catalog,
including parent-menu names, regardless of the displayed submenu. Native menus and
all frontend palettes share command display names and menu-family metadata.
Available selection commands and recent commands appear at the root and retain their normal routes. Menu-bar commands and palette routes are generated from the same catalog; platform-only palette commands are collected from the actual native menu leaves. Editing commands preserve the original window and focus.


About renders the original ANSI block grid as a single proportional graphic. It
scales to the content width without reflowing rows or depending on font metrics.

Enter in the Search input submits the query and keeps input focus. The input
keystroke is consumed before result navigation. Arrow navigation can move focus
to the results, where Enter opens the selected library destination.

Navigation and queue sidebars share a 280-point default width and the same
320 ms Kit spring; either remains manually resizable. Column filters are toggled
by the column title and expand into an inline field in the header when open. View Options
keeps control positions stable, disabling temporarily unavailable choices instead
of removing them. The transport information field keeps its size across panel and
screen transitions; its width responds only to the window width. Seeking never
changes the current screen, and Space toggles playback with either slider focused.
The transport up chevron and player down chevron provide explicit screen navigation.
Meter history occupies a narrow strip so short visualizer panes retain useful bar
height. The six visualizer modes are Waveform, Spectrum, Spectrogram, Vectorscope,
Landscape and Meters.

Explicitly named various-artists compilations with the same title and compatible
release years can span several provider album records. The shared compilation
index retains those member identities; browsing and queue loading assemble their
tracks rather than limiting the list to the current artist. Generic titles such
as Split remain separate without song-identity evidence. Singles classification
uses the assembled release count and omits artist excerpts.

The transport information field prefers 560 points and reserves room for the
remembered queue width even while the queue is hidden, so panel and screen changes
do not resize it. Narrow windows constrain its width to preserve the controls.
The transport up chevron and player down chevron use the same trailing inset and
button size; navigation is outside the lighter information field.

Play controls use click to play and Option-click to play next. Holding or right-clicking Play opens these choices. Track-and-following operations are available only in contextual menus. The transport uses 24-point playback
symbols in 36-point buttons. Twofer uses a medium-weight italic numeral.
The saved theme is published before creating the first window, and applied before
its controls are constructed. Music searches open the native app on macOS. A failed native handoff reports
an error and never redirects to a browser.

Selecting a similar album navigates to its Album Details destination with ancestor
selections restored. Similar Albums uses a track from that destination album,
including when the explicit details column has keyboard focus.

The Miller browser keeps its horizontal Kit scrollbar in a fixed 12-point bottom
gutter outside the scrolling columns. It stays visible while columns overflow.
Opening or replacing a destination scrolls to the right-hand limit after layout;
viewport changes keep the destination fully visible.

Column filter inputs include a small Kit close button in their suffix. Closing
clears the filter, restores the title, and returns keyboard focus to browsing.

Search windows present one results list. Opening a result uses the shared library
detail destination and restores ancestor selection instead of embedding a second
details pane in the search window. Local bundles use an available Apple development
certificate to keep macOS privacy authorization stable across builds;
`TEXTAMP_CODESIGN_IDENTITY` selects an explicit signing identity.

The terminal frontend projects the same source-aware sidebar groups, queue actions, library jumps and Journey endpoint choices. Its column title is the inline filter entry point; the title and header actions have a keyboard cursor as well as pointer targets. Content columns share available width equally and overflow horizontally below the terminal minimum width. The left sidebar remains fixed. Terminal transport controls use a five-row arrangement with centered metadata, a scrubber and elapsed/remaining time; notifications occupy the fixed bottom-right footer. External terminal image protocols use character rendering beneath modals and palettes; embedded Text retains images and clips their Unicode placements around overlays.

Terminal playback controls live in detail panes rather than ordinary list headers.
A terminal has no hover state; the footer names the focused control. Tab and
Shift+Tab traverse panes, with arrows selecting controls inside a pane. Sidebar
and queue visibility use `[` and `]`; `^` switches Library and Now Playing.
Shift-click or Alt-click opens a sidebar disclosure's children as a column.
Track-and-following operations are contextual command-palette choices. Detail
buttons always operate on the selected track. All presentations dispatch shared
library, queue and playback commands.
Describe Music keeps an inline query above the same history and result columns.

The terminal uses heavier single-line outlines for the sidebar and queue, with lighter single-line Miller
columns. Queue Type and Queue Tools share the queue header. The terminal opens
these menus through its contextual palette; embedded Text mode paints native line
icons over the terminal symbols. Tab switches panes; arrows select header menus,
transport and footer controls.
Position and volume sliders accept Left/Right. The track card
and chevron both toggle Library and Now Playing.

Terminal panel transitions use eased cell geometry and buffer composition, capped
at the existing 30-frame-per-second render cadence. They stop requesting frames
when settled. Drawer widths and the sliding Now Playing panel retain actual hit
regions. Artwork uses character rendering during motion, then restores the chosen
terminal protocol. `TERM=dumb` keeps transitions disabled.

Terminal navigation and queue panels use the theme's secondary surface and
heavier single-line frames; library content uses the primary surface and single-line
frames. Accent selection identifies keyboard focus, while a neutral retained
selection marks the navigation path. Playback mode buttons use accent only when
active. The track card has a separate surface with contrast-aware text, including
in monochrome themes. Inline filters belong to their selected column and do not
filter or remove adjacent columns.

Terminal album track lists expose a labeled Album Details row above the tracks.
Artist Details is an explicit destination in the artist's album list. Enter on
library items opens content or details without playback; explicit playback
controls and double-click retain playback behavior. Opening a details column
reveals its full width and gives it keyboard focus.

The TUI Settings surface is a centered popover over the current workspace. Its
category list and content panel retain keyboard and pointer navigation; the close
control exits the popover from either panel. Terminal artwork uses character
rendering while Settings is visible to keep images below the overlay.

Named palettes and track-card colors come from `app::theme_palette`, shared with
the terminal adapter. Both frontends offer artwork rendering choices. Artwork
visibility is shared by column type within each library.

The pinned GPUI Kit 0.9.0 facade and gpui-component 0.9.0 expose inputs, lists,
scrollbars and native window composition, but no VT terminal surface or Kitty
image placement renderer. Embedded Text mode therefore requires custom cell and
image painting from libghostty-vt render state; surrounding native controls and
scrollbars continue to use Kit. The host owns PNG decoding and glyph shaping.

## Embedded Text presentation

`app::runtime::Session` owns the live state, configuration, audio, playback clock,
event channel and task session. The desktop actor serializes both frontend inputs
and completions. Its projections and native window registry remain adapters;
logical workflows use shared state and `app::workflow::Context`.

`--text` creates no PTY or child player. Ratatui writes VT output to an in-process
sink with explicit cell/pixel metrics. Bundled libghostty-vt 1.0.1 parses it.
The host reads render-state rows, dirty flags, grapheme clusters, styles, cursor
and Kitty placements. It decodes PNG images and draws cached textures separately
from cell backgrounds and text. Unicode box rules and block graphics use continuous cell geometry, including
scrollbar thumbs and fractional artwork cells.
Kit 0.9.0 provides no terminal surface, so this drawing and native event adapter
are custom; surrounding window controls and native menus retain Kit.

The pinned Kit source has two narrow extensions: Settings selection feedback,
and opaque suspension/restoration of dialog builders, focused input and focus
handles without invoking dismissal callbacks. These preserve category selection
and unfinished dialogs across presentation changes. Native open/save panels and
active IME composition defer switching until they close or finish; the current
presentation remains active and a status message explains the wait.

The View menu exposes both windowed modes directly; ⌘⇧T and Ctrl+Shift+T
toggle them. The shared workspace identifies Library or Now Playing independently
of native window arrangement. Open track details and queue visibility also live
in shared state. Empty Now Playing uses a music-note placeholder on the left and
the same themed ANSI artwork on the right in both presentations.

Preparation retains the active view until a destination frame is ready. Input
and resize messages carry presentation identities; mouse targets also carry the
viewport revision. Only successful activation saves the windowed preference.
Inactive terminal rendering is suspended and disposable VT/image resources are
released. Native auxiliary windows and Kit dialogs are hidden, retained and
restored on return. Settings forms transfer privately, outside public snapshots.
Scroll continuity uses source-scoped item keys and clipped-item fractions;
frontend geometry remains local. Pointer pinning survives the transfer.

The macOS package statically links Ghostty and includes its MIT and simdutf
notices. The archive and selected static library checksums are recorded in
`vendor/ghostty-vt/SHA256SUMS`. No development toolchain is required at runtime.


### Presentation verification and concrete limits

Development checks use isolated XDG paths and the deterministic loopback catalog.
Native GUI → Text → GUI checks cover retained artist/album columns, native and
terminal palettes, Unicode paste with combining and wide characters, Settings
form drafts, Kitty artwork and repeated player/queue transitions. A native audio
check uses the loopback WAV fixture at zero volume: the clock continues in Text
mode and pausing there remains paused after returning to GUI. Terminal lifecycle
checks exercise normal quit and interruption with exact termios restoration.
Focused state tests cover stale input, viewport revisions, filtered anchors,
pending connection validation and presentation rollback.
Native Preview checks also cover the empty Now Playing artwork in Solarized Dark
and System appearance, View → Modes choices, both mode-switch shortcuts,
Settings category transfer and Unicode palette paste immediately after switching.
Native Preview checks at an 832-point window width cover both presentations with
the queue visible. The graphical visualizer tabs scroll horizontally while their
full-screen control remains visible. Native Option-E composition commits accented
text in the terminal palette, including field selection and deletion.

The terminal surface exposes native text editing and candidate-window bounds.
IME composition and marked text defer switching; a full native input-method matrix
has not been manually exercised. Single-line fields share grapheme-aware UTF-16
caret, selection and horizontal-scroll geometry with the native input adapter,
including column filters, Description, Journey and connection forms. Legacy
multiline input dialogs retain their terminal caret anchor. Terminal output
selection uses Command-drag; editable fields use the normal Edit-menu commands.

Each presentation independently selects Images, ANSI half blocks or Braille for
its library. Downloaded artwork and column-type visibility remain shared. Text placeholders use the same
Lucide note, disc and person shapes as the graphical frontend, through the chosen
artwork encoder. Embedded image uploads convert RGBA storage to GPUI's BGRA texture
format. Display-scale and input-method checks across the full supported hardware
matrix remain outstanding; deterministic resize checks do not substitute for them.

Presentation handoffs seed panel geometry at its destination, without replaying
sidebar, queue, player or column transitions. Navigation within a presentation
retains its normal animations. Embedded queue controls use the same monochrome
GPUI Kit icons as graphical controls, scaled to the terminal row. Standalone
terminals use Unicode text symbols instead of requiring a custom icon font.

Windowed graphical and Text presentations share transport/footer dimensions,
sidebar and queue widths, Now Playing header measurements and transport time
labels. Handoffs retain resized pane dimensions. Text remains monospace and GUI
uses the system font; terminal-cell rounding can differ by less than one cell.
Both frontends put album captions beside small artwork and below large, left-aligned artwork; captions include an available year. Large Artwork is the
same shared View Options setting in both frontends, retaining its saved key.
Both transports retain their thumbnail or note placeholder in Library and Now
Playing. Terminal placement protocols are recreated with a renderer, while
mode-independent decoded image caches remain reusable.
Embedded Text graphics mode uses the graphical visualizer painter within the
terminal's published canvas bounds. Half-block and braille artwork modes use the
Ratatui character visualizer. External terminal sessions retain character
visualizers without initializing GPUI.

Artwork rendering offers Images, ANSI half blocks and Braille in either frontend.
Graphical mode encodes character artwork into an SVG image for Kit's image widget;
braille cells share the terminal encoder. Rendered variants have a 32 MiB memory
budget. Encoding is saved separately for Graphical and Text mode per library;
column-type visibility is shared and defaults on. External terminal capability
fallbacks do not modify either saved encoding.

Kit dialog backdrops cover document content below the title bar. The title bar retains native dragging and window controls while an overlay is open.

Appearance uses a Kit list for theme previews. Highlighting changes only the sample; clicking the highlighted theme again or pressing Enter applies the shared preference.

Command-Option-T cycles GUI Multiple Windows, GUI Single Window, and Text through the shared Mode preference. It is also available in View and the windowed palettes.

The Kit list normally confirms rows on the first click. Theme rows intercept that click to preview a newly highlighted choice; a second click or Enter confirms it.

### Keyboard focus

Graphical mode uses Tab and Shift+Tab to traverse native controls, including the
transport sliders. Arrow keys navigate within the focused library list or control.
Text mode retains panel traversal with Tab and arrow navigation inside panels.
Application navigation bindings are defined in `src/app/shortcuts.rs`; native
adapters retain window activation while dispatching the same shared commands.
Settings dialogs let Kit capture the originating focus before focusing their
content, so closing the dialog returns to the control that opened it.

### Activity and compact layouts

Activity labels derive from shared request ownership. Background artwork tasks
alone do not keep a generic loading indicator active. Cached artist biographies
and related names remain readable while a refresh is pending. Failed related
music can be retried with F5 without replacing the browsed album or track list;
an empty failed library view also exposes a stock Kit Retry button.

The transport's shared width calculation reserves its two control groups before
assigning the track card's width, including narrow workspaces. Details retain
metadata beneath their existing kind heading without an additional Information
heading. The Kit dialog backdrop delegates input to Base's dismissal handler;
its painted surface does not occlude that handler or the title bar's drag area.

The monochrome theme outlines the transport card on white so the stock Kit slider remains legible. Active toolbar toggles use Kit primary buttons; normal and hovered controls retain contrasting text.

Playback controls perform their primary action on click. A stationary primary-button hold opens the same Kit context menu as right-click, with Play and Play Next; Option-click selects Play Next directly. The context-menu adapter opts into primary-button holds because Kit supplies right-click and touch long-press activation but no primary-mouse hold activation. Movement cancels the hold, and release after opening the menu is consumed. Track-and-following operations remain in contextual command menus.

External music searches resolve the visible detail, active artist or album column,
then playback. ⌘⇧A (Ctrl+Shift+A in the TUI) sends the Apple Music search URL
to the Music app using macOS Launch Services. Windows targets the installed
Apple Music package with the same URL. Other desktop platforms report Apple Music search as unavailable. Spotify prefers its app and desktop YouTube uses web results.
