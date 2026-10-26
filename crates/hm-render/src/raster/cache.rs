//! Decoded pictures shared by every page a process renders, bounded by the
//! bytes they hold: the least recently used go first, so a document full
//! of photographs keeps only what the pages being drawn need.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use tiny_skia::Pixmap;

use crate::image::Effects;

/// Bytes the cache may hold. A picture larger than half of it is never
/// kept.
pub(super) const BUDGET: usize = 256 << 20;

type Data = (usize, usize, usize);

/// What a cached pixmap is: picture data (by identity) with its effects,
/// decoded at its own size, played (metafiles) or averaged down to a size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Key {
    Decoded(Data, Effects),
    Played(Data, Effects, u32, u32),
    Shrunk(Data, Effects, u32, u32, bool),
}

impl Key {
    pub(super) fn data(data: &hm_model::SharedBytes) -> Data {
        data.identity()
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
    dims: HashMap<(Data, Effects), (u32, u32)>,
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
    if let (Key::Decoded(data, fx), Some(p)) = (key, &value) {
        if c.dims.len() > 4096 {
            c.dims.clear();
        }
        c.dims.insert((data, fx), (p.width(), p.height()));
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
pub(super) fn dims(data: Data, fx: Effects) -> Option<(u32, u32)> {
    lru().lock().unwrap().dims.get(&(data, fx)).copied()
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

/// Bytes that pictures being decoded at once may take. Pages render on
/// several threads; without a bound each could hold a large photograph's
/// pixels and its decoder's buffers at the same time.
const DECODING: usize = 256 << 20;

/// Runs `f`, a decode taking about `bytes`, once the decodes in flight
/// leave room for it. One decode always runs, however large.
pub(super) fn decoding<T>(bytes: usize, f: impl FnOnce() -> T) -> T {
    use std::sync::Condvar;
    static GATE: OnceLock<(Mutex<usize>, Condvar)> = OnceLock::new();
    let (lock, cv) = GATE.get_or_init(Default::default);
    {
        let mut busy = lock.lock().unwrap();
        while *busy > 0 && *busy + bytes > DECODING {
            busy = cv.wait(busy).unwrap();
        }
        *busy += bytes;
    }
    struct Done<'a>(&'a Mutex<usize>, &'a Condvar, usize);
    impl Drop for Done<'_> {
        fn drop(&mut self) {
            *self.0.lock().unwrap() -= self.2;
            self.1.notify_all();
        }
    }
    let _done = Done(lock, cv, bytes);
    f()
}
