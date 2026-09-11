# Input and split-view audit — 2026-09-06

Scope: keyboard ownership, mouse hit testing, split-view focus, popup capture,
filter lifecycle, queue selection, Help scrolling and provider-independent
navigation. This is not a certification that every application or server path
is bug-free. No live playback or remote-player control was exercised.

## Confirmed findings and changes

1. **Clicks could pass through text dialogs to the player.**
   `handlers/mouse_input.rs` did not capture mouse events for the single-field
   input dialog. Clicking, scrolling or opening a context menu could change the
   underlying queue. Text dialogs and error overlays now block those events and
   clear lingering drag operations.
2. **Displayed focus disagreed with keyboard routing.**
   `ui/app.rs` and `ui/screens/now_playing.rs` rendered retained library/queue
   selections as focused even when the other split pane owned input. Focused
   borders and selection styles now require the owning `View`; the visualizer
   has a visible focus border. No second top-level focus variable was added.
3. **Tab took different execution paths in opposite directions.**
   Browse directly mutated `view`, bypassing the navigation dispatch that loads
   analysis; playback views returned a navigation action. One shared Tab/Shift+Tab
   route now handles all browse categories and providers. Settings, search tabs
   and dialogs retain their own Tab navigation.
4. **A displayed queue was not necessarily clickable.**
   `handle_now_playing_down` returned immediately in the NowPlaying view even
   though that view renders the same queue. Pointer routing now distinguishes
   queue/artwork from visualizer using the renderer's actual rectangles.
   Subsequent arrow keys go to the clicked component.
5. **Artwork clicks could target queue rows.**
   The queue hit test checked only its left boundary, so the artwork to its right
   also qualified. Artwork now has its own focus destination; track clicks must
   be inside the track-list rectangle.
6. **Wheel and context-menu routing used stale keyboard focus or old geometry.**
   Wheel input now scrolls the pane under the pointer without stealing focus.
   Right clicks focus the actual pane, account for two terminal rows per queue
   track, and pin the current viewport. Visualizer-tab clicks also move keyboard
   focus to the tab bar.
7. **Global shortcuts intercepted captured text.**
   Search, authentication and inline filters could lose punctuation to palette,
   layout or navigation commands. Popup/text owners now run before those commands;
   standard quit remains available even during dialogs and errors. The TUI's
   duplicate palette router was removed. F1/F2/F3 remain usable at authentication,
   and the library switcher still accepts input there.
8. **View transitions left selection-mode state inconsistent.**
   `AppState::set_view` now ignores redundant focus requests, clears text capture
   and selection mode on real transitions, and preserves queue multi-selection
   while moving within the combined queue/visualizer screen.
9. **Obsolete stations-panel input was still active.**
   Wheel handling tested a station-panel position that is no longer rendered.
   Removed its keyboard navigation, click/scrollbar handlers, synthetic rectangles,
   focus variant and unused scroll state. The actual station browser, radio
   commands and sidebar remain supported. The removed focus enum was not serialized.
10. **Help could not reliably reach its ending.**
    Keyboard scrolling assumed 140 lines; mouse geometry assumed a different
    full-screen layout. Render feedback now supplies the real wrapped line count
    and viewport to keyboard, wheel and scrollbar handling. Page keys use the
    visible height. This opts into the existing Ratatui 0.29 dependency's rendered
    line-info API; it adds no package and avoids duplicating its wrapping algorithm.

The dead comma-seek branch was also removed: comma already opens Settings
globally. Shift+Left/Right seeking and the existing period shortcut are preserved.

## Complexity measurement

Lizard 1.24.0, identical command and file scope before and after:

```sh
lizard src/app/handlers/key_input/mod.rs \
  src/app/handlers/key_input/browse.rs \
  src/app/handlers/key_input/now_playing.rs \
  src/app/handlers/mouse_input.rs \
  src/app/handlers/key_input/settings.rs
```

| Function | Before CCN | After CCN |
| --- | ---: | ---: |
| `handle_now_playing_down` | 49 | 25 |
| `handle_scroll` | 15 | 11 |
| `handle_queue_keys` | 9 | 7 |
| `handle_station_keys` | 37 | Removed |
| `skip_station_separators` | 9 | Removed |
| `try_station_scrollbar_click` | 7 | Removed |

These are lexical, tool-reported cyclomatic-complexity estimates, not file-length
proxies or formal control-flow analysis. Rust macros are not expanded. Lizard
misidentifies the ending of the large `handle_key` function, so its apparent CCN
change is deliberately excluded. The reductions above principally remove actual
obsolete interaction paths; the new pointer-ownership helper has a concrete role
shared by clicking and scrolling.

## Verification

Baseline: full tests, strict all-target Clippy, formatting and release build
passed, with four opt-in tests skipped. Eight new regression tests initially
failed, reproducing the principal input/focus defects before implementation.
The regression suite then expanded to cover all four providers, authentication
escape routes, protected punctuation input, quit inside overlays, wrapped Help,
viewport-preserving context clicks, and view-transition cleanup. Existing seeking,
library-manager, visualizer, provider and persistence tests remain part of the
full suite.

The release PTY smoke uses private temporary copies of user settings and checks
actual visible WebDAV focus changes in split mode, along with the account/form
workflow, both WebDAV listings and terminal restoration. It never starts music,
signs out a live account, or modifies server data. Plex/Navidrome input routing is
covered with fixtures; live audio hardware, remote playback, every terminal theme
and every possible modifier encoding are outside this verification scope.

Final results:

- `cargo test`: 360 passing test results (including child-fixture results), zero
  failures, four opt-in tests ignored. Fifteen focused input regression tests pass.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo fmt -- --check` and `git diff --check`: passed.
- `cargo build --release --bin textamp`: passed with no warnings.
- Release PTY smoke: passed Settings/account/form navigation, both live WebDAV
  listings, repeated visible split-focus round trips, mixed-source switcher,
  clean shutdown and terminal restoration. The fixture removes inherited
  `NO_COLOR` so its focus-color assertions actually measure rendered colors.
- Installed atomically to `/Users/bergmayer/Applications/textamp`, without an
  old-build backup. Build and installed SHA-256 both:
  `a20fb5efb5449d59984712af4aef915d9a51ffa1851906467f6c022ddc606e72`.

Verification logs are in `/tmp/textamp-focus-final-tests.log`,
`/tmp/textamp-focus-final-clippy.log`, `/tmp/textamp-focus-release.log` and
`/tmp/textamp-focus-tui.log`. No user configuration, live library content or
account credentials were changed by this audit. No commit was made.
