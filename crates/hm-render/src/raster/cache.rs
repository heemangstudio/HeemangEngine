//! Decoded pictures shared by every page a process renders, bounded by the
//! bytes they hold: the least recently used go first, so a document full
//! of photographs keeps only what the pages being drawn need.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use tiny_skia::Pixmap;

use crate::image::Effects;

/// Bytes the cache may hold. A picture larger than half of it is never
/// kept.
const BUDGET: usize = 256 << 20;

/// What a cached pixmap is: picture data (by identity) with its effects,
/// decoded at its own size, played (metafiles) or averaged down to a size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Key {
    Decoded(usize, usize, Effects),
    Played(usize, usize, Effects, u32, u32),
    Shrunk(usize, usize, Effects, u32, u32),
}

impl Key {
    pub(super) fn data(data: &hm_model::SharedBytes) -> (usize, usize) {
        (data.as_ptr() as usize, data.len())
    }
}

struct Entry {
    value: Option<Arc<Pixmap>>,
    bytes: usize,
    used: u64,
}

#[derive(Default)]
struct Lru {
    map: HashMap<Key, Entry>,
    /// Pixel sizes of decoded pictures, kept after their pixels go.
    dims: HashMap<(usize, usize, Effects), (u32, u32)>,
    bytes: usize,
    tick: u64,
}

fn lru() -> &'static Mutex<Lru> {
    static CACHE: OnceLock<Mutex<Lru>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The cached pixmap for `key`, or `make`'s, kept when it fits the budget.
/// `None` results (undecodable data) are kept too, at no cost.
pub(super) fn get_or(key: Key, make: impl FnOnce() -> Option<Pixmap>) -> Option<Arc<Pixmap>> {
    {
        let mut c = lru().lock().unwrap();
        c.tick += 1;
        let tick = c.tick;
        if let Some(e) = c.map.get_mut(&key) {
            e.used = tick;
            return e.value.clone();
        }
    }
    let value = make().map(Arc::new);
    let bytes = value.as_ref().map_or(0, |p| p.data().len());
    let mut c = lru().lock().unwrap();
    if let (Key::Decoded(a, b, fx), Some(p)) = (key, &value) {
        if c.dims.len() > 4096 {
            c.dims.clear();
        }
        c.dims.insert((a, b, fx), (p.width(), p.height()));
    }
    if bytes > BUDGET / 2 {
        return value;
    }
    while c.bytes + bytes > BUDGET {
        let Some(oldest) = c.map.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| *k) else {
            break;
        };
        if let Some(e) = c.map.remove(&oldest) {
            c.bytes -= e.bytes;
        }
    }
    c.tick += 1;
    let used = c.tick;
    c.bytes += bytes;
    c.map.insert(
        key,
        Entry {
            value: value.clone(),
            bytes,
            used,
        },
    );
    value
}

/// The pixel size of a picture decoded before, if remembered.
pub(super) fn dims(data: (usize, usize), fx: Effects) -> Option<(u32, u32)> {
    lru()
        .lock()
        .unwrap()
        .dims
        .get(&(data.0, data.1, fx))
        .copied()
}

/// The cached pixmap for `key`, without making it.
pub(super) fn peek(key: Key) -> Option<Option<Arc<Pixmap>>> {
    let mut c = lru().lock().unwrap();
    c.tick += 1;
    let tick = c.tick;
    c.map.get_mut(&key).map(|e| {
        e.used = tick;
        e.value.clone()
    })
}
