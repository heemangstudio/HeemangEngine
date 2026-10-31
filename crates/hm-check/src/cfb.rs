//! Strict compound file reading: every header field, FAT, DIFAT, mini FAT
//! and directory chain, sector ownership and the directory trees.

use crate::{Report, u16_at, u32_at, u64_at};

const MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const FREE: u32 = 0xFFFF_FFFF;
const END: u32 = 0xFFFF_FFFE;
const FAT_SECT: u32 = 0xFFFF_FFFD;
const DIFAT_SECT: u32 = 0xFFFF_FFFC;
const MAX_REG: u32 = 0xFFFF_FFFA;
const NO_STREAM: u32 = 0xFFFF_FFFF;
const CUTOFF: u64 = 4096;
const MINI: usize = 64;
const A: &str = "cfb";

/// A stream reached from the root, by full path (`BodyText/Section0`).
pub struct Stream {
    pub path: String,
    pub data: Vec<u8>,
}

struct Entry {
    name: Vec<u16>,
    kind: u8,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    size: u64,
}

/// Records that `who` owns sector `s`; a sector owned twice is an error.
fn claim(owners: &mut [Option<String>], s: u32, who: &str, r: &mut Report) {
    let Some(slot) = owners.get_mut(s as usize) else {
        return;
    };
    if let Some(prev) = slot {
        r.error(A, format!("sector {s} belongs to both {prev} and {who}"));
    } else {
        *slot = Some(who.to_string());
    }
}

/// Follows a chain from `start`; errors name `who`.
fn chain(next: &[u32], start: u32, limit: usize, who: &str, r: &mut Report) -> Option<Vec<u32>> {
    let mut out = Vec::new();
    let mut seen = vec![false; limit];
    let mut s = start;
    while s != END {
        if s as usize >= limit || s > MAX_REG {
            r.error(
                A,
                format!("{who}: chain reaches sector {s:#x} outside the file"),
            );
            return None;
        }
        if std::mem::replace(&mut seen[s as usize], true) {
            r.error(A, format!("{who}: chain loops at sector {s}"));
            return None;
        }
        out.push(s);
        let Some(&n) = next.get(s as usize) else {
            r.error(A, format!("{who}: sector {s} has no allocation entry"));
            return None;
        };
        if n == FREE {
            r.error(A, format!("{who}: chain runs into free sector {s}"));
            return None;
        }
        s = n;
    }
    Some(out)
}

fn upper(u: u16) -> u16 {
    char::from_u32(u as u32)
        .and_then(|c| {
            let mut up = c.to_uppercase();
            let first = up.next()?;
            (up.next().is_none() && (first as u32) <= 0xFFFF).then_some(first as u16)
        })
        .unwrap_or(u)
}

/// Sibling order: shorter names first, then unit by unit in upper case.
fn order(a: &[u16], b: &[u16]) -> std::cmp::Ordering {
    a.len()
        .cmp(&b.len())
        .then_with(|| a.iter().map(|&u| upper(u)).cmp(b.iter().map(|&u| upper(u))))
}

/// Reads a compound file strictly. Returns every stream reached from the
/// root, or `None` when the file is too damaged to walk.
pub fn read(data: &[u8], r: &mut Report) -> Option<Vec<Stream>> {
    if data.len() < 512 || data[..8] != MAGIC {
        r.error(A, "missing compound file signature");
        return None;
    }
    let h = |o| u16_at(data, o).unwrap();
    let w = |o| u32_at(data, o).unwrap();
    let major = h(26);
    let shift = h(30);
    if h(28) != 0xFFFE {
        r.error(A, format!("byte order mark {:#06x}", h(28)));
    }
    let ss = match (major, shift) {
        (3, 9) => 512,
        (4, 12) => 4096,
        _ => {
            r.error(A, format!("version {major} with sector shift {shift}"));
            return None;
        }
    };
    if h(32) != 6 {
        r.error(A, format!("mini sector shift {}", h(32)));
    }
    if w(56) != CUTOFF as u32 {
        r.error(A, format!("mini stream cutoff {}", w(56)));
    }
    if major == 3 && w(40) != 0 {
        r.error(A, "version 3 file declares directory sectors");
    }
    if data.len() < ss {
        r.error(A, "file shorter than its header sector");
        return None;
    }
    if data.len() % ss != 0 {
        r.warn(
            A,
            format!(
                "file length {} is not a whole number of sectors",
                data.len()
            ),
        );
    }
    let nsec = (data.len() - ss).div_ceil(ss);
    let sector = |s: u32| -> Vec<u8> {
        let start = (s as usize + 1) * ss;
        let mut v = data
            .get(start..(start + ss).min(data.len()))
            .unwrap_or(&[])
            .to_vec();
        v.resize(ss, 0);
        v
    };
    let words = |b: &[u8]| -> Vec<u32> {
        b.chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect()
    };

    // DIFAT: 109 header slots, then the DIFAT sector chain.
    let nfat = w(44) as usize;
    let ndifat = w(72) as usize;
    let mut fat_ids = Vec::new();
    for i in 0..109 {
        let v = w(76 + 4 * i);
        if i < nfat {
            fat_ids.push(v);
        } else if v != FREE {
            r.warn(A, format!("unused header DIFAT slot {i} holds {v:#x}"));
        }
    }
    let mut difat_ids = Vec::new();
    let mut d = w(68);
    while d != END && d != FREE {
        if difat_ids.len() >= ndifat {
            r.error(
                A,
                format!("DIFAT chain longer than the {ndifat} sectors declared"),
            );
            break;
        }
        if d as usize >= nsec || difat_ids.contains(&d) {
            r.error(A, format!("DIFAT chain reaches bad sector {d:#x}"));
            break;
        }
        difat_ids.push(d);
        let sec = words(&sector(d));
        for (j, &v) in sec[..sec.len() - 1].iter().enumerate() {
            if fat_ids.len() < nfat {
                fat_ids.push(v);
            } else if v != FREE {
                r.warn(
                    A,
                    format!("unused DIFAT slot {j} of sector {d} holds {v:#x}"),
                );
            }
        }
        d = *sec.last().unwrap();
    }
    if difat_ids.len() != ndifat {
        r.error(
            A,
            format!("{} DIFAT sectors found, {ndifat} declared", difat_ids.len()),
        );
    }
    if fat_ids.len() != nfat {
        r.error(
            A,
            format!("{} FAT sectors listed, {nfat} declared", fat_ids.len()),
        );
    }
    if let Some(bad) = fat_ids.iter().find(|&&s| s as usize >= nsec) {
        r.error(A, format!("FAT sector {bad:#x} lies outside the file"));
        return None;
    }
    let fat: Vec<u32> = fat_ids.iter().flat_map(|&s| words(&sector(s))).collect();
    if fat.len() < nsec {
        r.error(
            A,
            format!("FAT covers {} sectors of {nsec} in the file", fat.len()),
        );
    }
    for &s in &fat_ids {
        if fat.get(s as usize) != Some(&FAT_SECT) {
            r.error(A, format!("FAT sector {s} is not marked as a FAT sector"));
        }
    }
    for &s in &difat_ids {
        if fat.get(s as usize) != Some(&DIFAT_SECT) {
            r.error(
                A,
                format!("DIFAT sector {s} is not marked as a DIFAT sector"),
            );
        }
    }
    if let Some(i) = (nsec..fat.len()).find(|&i| fat[i] != FREE) {
        r.warn(
            A,
            format!("FAT entry {i} past the end of the file is in use"),
        );
    }
    let marked_fat = fat.iter().filter(|&&v| v == FAT_SECT).count();
    if marked_fat != nfat {
        r.error(
            A,
            format!("{marked_fat} sectors marked as FAT, {nfat} declared"),
        );
    }

    let mut owners: Vec<Option<String>> = vec![None; nsec];
    for &s in &fat_ids {
        claim(&mut owners, s, "the FAT", r);
    }
    for &s in &difat_ids {
        claim(&mut owners, s, "the DIFAT", r);
    }
    let dir_chain = chain(&fat, w(48), nsec, "directory", r)?;
    for &s in &dir_chain {
        claim(&mut owners, s, "the directory", r);
    }
    let dir: Vec<u8> = dir_chain.iter().flat_map(|&s| sector(s)).collect();
    let entries: Vec<Entry> = dir
        .chunks_exact(128)
        .map(|e| Entry {
            name: {
                let n = (u16_at(e, 64).unwrap() as usize / 2).clamp(1, 32) - 1;
                (0..n).map(|k| u16_at(e, 2 * k).unwrap()).collect()
            },
            kind: e[66],
            left: u32_at(e, 68).unwrap(),
            right: u32_at(e, 72).unwrap(),
            child: u32_at(e, 76).unwrap(),
            start: u32_at(e, 116).unwrap(),
            size: u64_at(e, 120).unwrap(),
        })
        .collect();
    for (i, e) in dir.chunks_exact(128).enumerate() {
        let kind = e[66];
        if !matches!(kind, 0 | 1 | 2 | 5) {
            r.error(A, format!("directory entry {i} has kind {kind}"));
        }
        if kind == 5 && i != 0 {
            r.error(A, format!("directory entry {i} is a second root"));
        }
        if kind == 0 {
            continue;
        }
        let len = u16_at(e, 64).unwrap() as usize;
        if len % 2 != 0 || !(2..=64).contains(&len) || u16_at(e, len - 2) != Some(0) {
            r.error(
                A,
                format!("directory entry {i} has a bad name length {len}"),
            );
        }
        let name = &entries[i].name;
        if name
            .iter()
            .any(|&u| matches!(u, 0x2F | 0x5C | 0x3A | 0x21) || u == 0)
        {
            let n = String::from_utf16_lossy(name);
            r.error(
                A,
                format!("directory entry {i} name {n:?} has a reserved character"),
            );
        }
        if major == 3 && kind == 2 && entries[i].size >> 32 != 0 {
            r.error(
                A,
                format!("directory entry {i} size exceeds 4 GiB in a version 3 file"),
            );
        }
    }
    if entries.first().map(|e| e.kind) != Some(5) {
        r.error(A, "directory entry 0 is not the root");
        return None;
    }

    // Mini FAT and mini stream.
    let nminifat = w(64) as usize;
    let minifat_start = w(60);
    let minifat_chain = if minifat_start == END || (nminifat == 0 && minifat_start == FREE) {
        Vec::new()
    } else {
        chain(&fat, minifat_start, nsec, "mini FAT", r).unwrap_or_default()
    };
    if minifat_chain.len() != nminifat {
        let msg = format!(
            "mini FAT chain has {} sectors, {nminifat} declared",
            minifat_chain.len()
        );
        r.error(A, msg);
    }
    for &s in &minifat_chain {
        claim(&mut owners, s, "the mini FAT", r);
    }
    let minifat: Vec<u32> = minifat_chain
        .iter()
        .flat_map(|&s| words(&sector(s)))
        .collect();
    let root = &entries[0];
    let mini_chain = if root.size == 0 {
        Vec::new()
    } else {
        chain(&fat, root.start, nsec, "mini stream", r).unwrap_or_default()
    };
    if mini_chain.len() < (root.size as usize).div_ceil(ss) {
        let msg = format!(
            "mini stream chain has {} sectors for {} bytes",
            mini_chain.len(),
            root.size
        );
        r.error(A, msg);
    }
    for &s in &mini_chain {
        claim(&mut owners, s, "the mini stream", r);
    }
    let mini: Vec<u8> = mini_chain.iter().flat_map(|&s| sector(s)).collect();
    let mini_sectors = (root.size as usize).min(mini.len()) / MINI;
    let mut mini_owners: Vec<Option<String>> = vec![None; mini_sectors];

    // Directory trees from the root.
    let mut paths: Vec<(String, usize)> = Vec::new();
    let mut reached = vec![false; entries.len()];
    reached[0] = true;
    let mut pending = vec![(String::new(), entries[0].child)];
    while let Some((prefix, top)) = pending.pop() {
        // In-order walk of one storage's sibling tree.
        let mut order_list = Vec::new();
        let mut stack = Vec::new();
        let mut cur = top;
        loop {
            while cur != NO_STREAM {
                if cur as usize >= entries.len() {
                    r.error(A, format!("directory link to missing entry {cur}"));
                    break;
                }
                if std::mem::replace(&mut reached[cur as usize], true) {
                    r.error(A, format!("directory entry {cur} is reached twice"));
                    break;
                }
                stack.push(cur);
                cur = entries[cur as usize].left;
            }
            let Some(n) = stack.pop() else {
                break;
            };
            order_list.push(n as usize);
            cur = entries[n as usize].right;
        }
        for pair in order_list.windows(2) {
            let (a, b) = (&entries[pair[0]].name, &entries[pair[1]].name);
            if order(a, b) != std::cmp::Ordering::Less {
                let msg = format!(
                    "siblings {:?} and {:?} under {:?} are out of order or equal",
                    String::from_utf16_lossy(a),
                    String::from_utf16_lossy(b),
                    if prefix.is_empty() { "/" } else { &prefix }
                );
                r.error(A, msg);
            }
        }
        for i in order_list {
            let e = &entries[i];
            let name = String::from_utf16_lossy(&e.name);
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            match e.kind {
                1 => pending.push((path.clone(), e.child)),
                2 if e.child != NO_STREAM => {
                    r.error(A, format!("stream {path:?} has children"));
                }
                0 | 5 => r.error(A, format!("tree reaches entry {i} of kind {}", e.kind)),
                _ => {}
            }
            paths.push((path, i));
        }
    }
    for (i, e) in entries.iter().enumerate() {
        if e.kind != 0 && !reached[i] {
            let n = String::from_utf16_lossy(&e.name);
            r.warn(A, format!("directory entry {i} ({n:?}) is not in the tree"));
        }
    }

    let mut streams = Vec::new();
    for (path, i) in paths {
        let e = &entries[i];
        if e.kind != 2 {
            continue;
        }
        let size = e.size as usize;
        let data = if e.size == 0 {
            Vec::new()
        } else if e.size < CUTOFF {
            let Some(c) = chain(&minifat, e.start, mini_sectors, &path, r) else {
                continue;
            };
            if c.len() < size.div_ceil(MINI) {
                let msg = format!("{path}: mini chain of {} for {size} bytes", c.len());
                r.error(A, msg);
                continue;
            }
            if c.len() > size.div_ceil(MINI) {
                r.warn(A, format!("{path}: mini chain longer than its size"));
            }
            let mut v = Vec::with_capacity(size);
            for &m in &c {
                if let Some(prev) = &mini_owners[m as usize] {
                    let msg = format!("mini sector {m} belongs to both {prev} and {path}");
                    r.error(A, msg);
                }
                mini_owners[m as usize] = Some(path.clone());
                v.extend_from_slice(&mini[m as usize * MINI..(m as usize + 1) * MINI]);
            }
            v.truncate(size);
            v
        } else {
            let Some(c) = chain(&fat, e.start, nsec, &path, r) else {
                continue;
            };
            if c.len() < size.div_ceil(ss) {
                let msg = format!("{path}: chain of {} sectors for {size} bytes", c.len());
                r.error(A, msg);
                continue;
            }
            if c.len() > size.div_ceil(ss) {
                r.warn(A, format!("{path}: chain longer than its size"));
            }
            for &s in &c {
                claim(&mut owners, s, &path, r);
            }
            let mut v: Vec<u8> = c.iter().flat_map(|&s| sector(s)).collect();
            v.truncate(size);
            v
        };
        streams.push(Stream { path, data });
    }
    // Sectors the FAT marks as used must belong to something.
    let lost = (0..nsec.min(fat.len()))
        .filter(|&s| fat[s] != FREE && owners[s].is_none())
        .count();
    if lost > 0 {
        r.warn(A, format!("{lost} allocated sectors belong to no stream"));
    }
    Some(streams)
}
