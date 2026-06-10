# Textamp Project Instructions

## Build Requirements

Build with `cargo build --release --bin textamp`. Do not say "done" until the build succeeds with zero warnings.

## UI Consistency

When making any functionality changes to the app, always update all relevant:
- Shortcut bars in `src/ui/app.rs` (`render_shortcuts()`)
- Help screen in `src/ui/screens/help.rs`
- Layout diagrams in code comments
- README.md if applicable

Clicking an item to highlight it should never recenter/scroll the view. Use the `scroll_pin` pattern to preserve viewport position on click — set pin on click, clear on keyboard navigation.

## Architecture

### Layer Separation

The UI is separate from the application logic. Each layer has clear boundaries:

```
┌─────────────────────────────────────────────────────────────┐
│                         UI Layer                             │
│  src/ui/ - ratatui renderer (pure render-from-state)        │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    App Core (orchestration)                  │
│  src/app/ - State, Events, Actions, Event Loop              │
└─────────────────────────────────────────────────────────────┘
                              │
        ┌─────────────────────┼─────────────────────┐
        ▼                     ▼                     ▼
┌───────────────┐   ┌─────────────────┐   ┌───────────────────┐
│ Plex Module   │   │ Services        │   │ Audio             │
│ src/plex/     │   │ src/services/   │   │ src/audio/        │
│               │   │                 │   │                   │
│ • PlexClient  │   │ • PlaybackSvc   │   │ • AudioBackend    │
│ • PlexAuth    │   │ • FolderSvc     │   │   (trait)         │
│ • Models      │   │ • LibrarySvc    │   │ • RodioBackend    │
│ • LibraryCache│   │ • Adventure     │   │ • DummyBackend    │
│ • WaveformCch │   │                 │   │                   │
└───────────────┘   └─────────────────┘   └───────────────────┘
```

The UI layer only consumes state; it never owns or mutates it. App-core
modules never import from `src/ui/`. Keeping this boundary clean is the
point of the separation.

### Plex Module (`src/plex/`)
The unified Plex integration layer. **No UI or audio imports.**

Structure:
- `mod.rs` - PlexService facade combining client + cache + preloading
- `client.rs` - HTTP API client
- `auth.rs` - Authentication (password, PIN/OAuth)
- `cache.rs` - LibraryCache for fast startup
- `waveform.rs` - Waveform generation and caching
- `models/` - All Plex data models
- `constants.rs` - API endpoints, headers, type IDs
- `error.rs` - Error types

### Audio (`src/audio/`)
- `AudioBackend` trait defines the interface
- `RodioBackend` implements it via rodio/symphonia
- `DummyBackend` for testing without audio hardware
- **No API or UI imports**

### Services (`src/services/`)
- Reusable business logic, **UI-agnostic**
- `PlaybackService` - queue management, track navigation
- `FolderService` - folder/file browsing
- `LibraryService` - library browse modes
- `generate_adventure` - sonic adventure algorithm
- Pure functions where possible, easily testable

### UI (`src/ui/`)
- Pure rendering from state — no mutation
- Imports data models from Plex (not the client)

### App Core (`src/app/`)
- `AppState` - single source of truth
- `EventLoop` - orchestrates all layers
- Elm Architecture (TEA) pattern

## Elm Architecture (TEA) Pattern

- **State**: Single `AppState` struct in `src/app/state.rs`
- **Events**: User input and async completions in `src/app/event.rs`
- **Actions**: Commands that modify state in `src/app/action.rs`
- **Render**: Pure function `ui::render(&Frame, &AppState)` in `src/ui/`

## Categories

Four browse categories accessible globally via Ctrl+key:
- Artists (Ctrl+A)
- Playlists (Ctrl+P) - tabbed: Playlists / Stations
- Genres (Ctrl+G) - tabbed: All / Library / Artist / Album / Mood / Style
- Folders (Ctrl+O) - Miller columns navigation

Albums are accessed by drilling into an Artist, Genre, or Mood.

## Views

- **Browse**: Main view showing categories and content
- **Search** (Ctrl+F): Tabbed search/filter view
- **Queue** (Ctrl+U): Track list with stations panel and artwork
- **Now Playing** (Ctrl+N): Visualizer with artwork, track info, and waveform/spectrum/spectrogram tabs
- **Similar**: Shows similar albums/tracks
- **Help** (F1): Keyboard shortcuts (scrollable)
- **Settings** (F2): Configuration

## Testing

Run tests with: `cargo test`

The `PlaybackService` has unit tests demonstrating testable service design.
Use `DummyBackend` for testing without audio hardware.

## File Locations

The app checks XDG environment variables first, then falls back to platform defaults.

### Config & Data Files

| File | XDG Override | Linux Default | macOS Default |
|------|--------------|---------------|---------------|
| Config | `$XDG_CONFIG_HOME/textamp/config.toml` | `~/.config/textamp/config.toml` | `~/Library/Application Support/textamp/config.toml` |
| Auth | `$XDG_DATA_HOME/textamp/auth.toml` | `~/.local/share/textamp/auth.toml` | `~/Library/Application Support/textamp/auth.toml` |
| Log | `$XDG_STATE_HOME/textamp/textamp.log` | `~/.local/state/textamp/textamp.log` | `~/Library/Application Support/textamp/textamp.log` |

### Cache Files

| File | XDG Override | Linux Default | macOS Default |
|------|--------------|---------------|---------------|
| Library cache | `$XDG_CACHE_HOME/textamp/library_*.json` | `~/.cache/textamp/library_*.json` | `~/Library/Caches/textamp/library_*.json` |
| Waveforms | `$XDG_CACHE_HOME/textamp/waveforms/*.json` | `~/.cache/textamp/waveforms/*.json` | `~/Library/Caches/textamp/waveforms/*.json` |

### Cache Settings

- **Library cache**: ~19MB per library
  - Per-category timestamps: each of the 11 RefreshCategory variants tracks its own age
  - Tier 1 (72h): Active category refreshed on view navigation if >72h old
  - Tier 2 (32d): Other categories refreshed on view navigation if >32 days old
  - Manual refresh: F5 refreshes current view
  - Stores: artists, albums, playlists, genres, stations, folders

- **Waveform cache**:
  - TTL: 7 days
  - Max size: 100 MB
  - ~8-15 KB per track
