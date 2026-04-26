//! Convert cached album-art bytes into `iced::image::Handle` values.
//!
//! `AppState::artwork::grid_cache` stores raw image bytes keyed by the
//! Plex rating key. The artwork subsystem preloads these in background;
//! the view function just hands each cached byte slice to Iced.
//!
//! Performance note: each call to `image::Handle::from_bytes` allocates
//! a fresh handle ID and forces Iced's image loader to decode the JPEG
//! pixels again. Rebuilding handles on every frame (the view closure
//! runs on every message) turned a Miller column with ~6 album-art
//! rows into a continuous decode loop that kept scrolling framerates
//! in the single digits. We cache the `Handle` per (key, bytes-slice)
//! identity so the decode happens exactly once and subsequent lookups
//! are just a hashmap get + cheap handle clone.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use iced::widget::image;

/// Identity of a cache entry. `ptr` and `len` together uniquely
/// identify the byte slice currently at a given key — if the byte cache
/// replaces the Vec (e.g., hi-res artwork loaded), either the pointer
/// or the length changes and the stale handle is evicted.
#[derive(Hash, Eq, PartialEq, Clone, Copy)]
struct SliceId {
    ptr: usize,
    len: usize,
}

struct CachedHandle {
    id: SliceId,
    handle: image::Handle,
}

fn handle_cache() -> &'static Mutex<HashMap<String, CachedHandle>> {
    static HANDLE_CACHE: OnceLock<Mutex<HashMap<String, CachedHandle>>> = OnceLock::new();
    HANDLE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Wrap a byte slice in an Iced image handle. Memoized by the slice's
/// (ptr, len) so re-rendering the same Vec<u8> across frames reuses
/// the decoded pixels instead of forcing Iced to decode again.
pub fn handle_from_bytes(bytes: &[u8]) -> image::Handle {
    let id = SliceId { ptr: bytes.as_ptr() as usize, len: bytes.len() };
    // Synthetic key — unique per slice identity, so concurrent art
    // for different Vecs doesn't collide.
    let key = format!("__raw_{}_{}", id.ptr, id.len);
    if let Ok(mut handles) = handle_cache().lock() {
        if let Some(entry) = handles.get(&key) {
            if entry.id == id {
                return entry.handle.clone();
            }
        }
        let handle = image::Handle::from_bytes(bytes.to_vec());
        handles.insert(key, CachedHandle { id, handle: handle.clone() });
        return handle;
    }
    image::Handle::from_bytes(bytes.to_vec())
}

/// Look up cached artwork for a rating key, returning the handle if
/// decoded bytes are available. Memoized: a given `(key, bytes)` slice
/// creates one handle that is reused across frames.
pub fn lookup_grid(
    cache: &HashMap<String, Vec<u8>>,
    key: &str,
) -> Option<image::Handle> {
    let bytes = cache.get(key)?;
    let id = SliceId { ptr: bytes.as_ptr() as usize, len: bytes.len() };

    let mut handles = handle_cache().lock().ok()?;
    if let Some(entry) = handles.get(key) {
        if entry.id == id {
            return Some(entry.handle.clone());
        }
    }
    let handle = image::Handle::from_bytes(bytes.clone());
    handles.insert(key.to_string(), CachedHandle { id, handle: handle.clone() });
    Some(handle)
}

/// Drop the GUI handle cache entirely. Called when the byte cache
/// itself is invalidated (e.g., user clears the artwork cache) so stale
/// decoded images don't linger.
pub fn clear_handle_cache() {
    if let Ok(mut h) = handle_cache().lock() {
        h.clear();
    }
}
