# Textamp Project Instructions

## Build Requirements

Always run `cargo build --release` before saying a task is complete. Do not say "done" until the release build succeeds.

## UI Consistency

When making any functionality changes to the app, always update all relevant:
- Shortcut bars in `src/ui/app.rs` (`render_shortcuts()`)
- Help screen in `src/ui/screens/help.rs`
- Layout diagrams in code comments
- README.md if applicable

## Architecture

### Layer Separation (Critical for Portability)

The codebase is designed for cross-platform portability. Each layer has clear boundaries:

```
┌─────────────────────────────────────────────────────────────┐
│                      UI Layer (replaceable)                  │
│  src/ui/ - Ratatui TUI (could be SwiftUI, Svelte, etc.)    │
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

### Plex Module (`src/plex/`)
The unified Plex integration layer. **This is the portable core for other platforms.**

Structure:
- `mod.rs` - PlexService facade combining client + cache + preloading
- `client.rs` - HTTP API client
- `auth.rs` - Authentication (password, PIN/OAuth)
- `cache.rs` - LibraryCache for fast startup
- `waveform.rs` - Waveform generation and caching
- `models/` - All Plex data models
- `constants.rs` - API endpoints, headers, type IDs
- `error.rs` - Error types

Key features:
- **No UI or audio imports** - fully portable
- Aggressive caching with TTL-based expiration
- Waveform cache with size limits and pruning
- Can be compiled as a library for FFI to Swift/Kotlin

Backward compatibility: `src/lib.rs` provides `api::` and `cache::` aliases that redirect to `plex::`.

### Audio (`src/audio/`)
- `AudioBackend` trait defines the interface
- `RodioBackend` implements it for desktop (rodio/symphonia)
- `DummyBackend` for testing without audio hardware
- **No API or UI imports**
- To port: implement `AudioBackend` for platform (AVFoundation, Web Audio)

### Services (`src/services/`)
- Reusable business logic, **UI-agnostic**
- `PlaybackService` - queue management, track navigation
- `FolderService` - folder/file browsing
- `LibraryService` - library browse modes
- `generate_adventure` - sonic adventure algorithm
- Pure functions where possible, easily testable

### UI (`src/ui/`)
- Pure rendering from state
- Only imports data models from Plex (not the client)
- Can be replaced entirely for different platforms

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
- **Now Playing** (Ctrl+N): Queue and playback status
- **Similar**: Shows similar albums/tracks
- **Help** (F1): Keyboard shortcuts (scrollable)
- **Settings** (F2): Configuration

## Testing

Run tests with: `cargo test`

The `PlaybackService` has unit tests demonstrating testable service design.
Use `DummyBackend` for testing without audio hardware.

## Porting Guidelines

When porting to a new platform:

1. **Keep**: `src/plex/`, `src/services/` (compile as library)
2. **Replace**: `src/ui/` with platform UI (SwiftUI, Svelte, etc.)
3. **Implement**: `AudioBackend` trait for platform audio
4. **Adapt**: `src/app/` event loop to platform patterns (may need significant changes)

The Plex module and services layers should work with minimal changes.

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
  - Per-category timestamps: each of the 13 RefreshCategory variants tracks its own age
  - Tier 1 (72h): Active category refreshed on view navigation if >72h old
  - Tier 2 (32d): Other categories refreshed on view navigation if >32 days old
  - Manual refresh: F5 refreshes current view
  - Stores: artists, albums, playlists, genres, stations, folders

- **Waveform cache**:
  - TTL: 7 days
  - Max size: 100 MB
  - ~8-15 KB per track
