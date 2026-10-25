//! Compound File Binary reading and writing.
//!
//! The reader is tolerant in the ways Hancom is: surplus FAT sectors, chains
//! that revisit a sector and damaged directory trees still open, while every
//! count and chain is bounded before it drives an allocation. The writer
//! emits version 3 files (512-byte sectors) that keep the source directory:
//! names, tree links, colors, CLSIDs, state bits and timestamps.

use std::collections::HashSet;

use crate::Error;

const MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const FREE: u32 = 0xFFFF_FFFF;
const END: u32 = 0xFFFF_FFFE;
const FAT_SECT: u32 = 0xFFFF_FFFD;
const DIFAT_SECT: u32 = 0xFFFF_FFFC;
const MINI_CUTOFF: u64 = 4096;
const MINI_SECTOR: usize = 64;
const MAX_ENTRIES: usize = 4096;
const MAX_DIFAT: usize = 128;
const MAX_TOTAL: u64 = 512 << 20;

/// Directory entry kinds.
pub const STORAGE: u8 = 1;
pub const STREAM: u8 = 2;
pub const ROOT: u8 = 5;

/// One directory entry as stored.
#[derive(Clone, Debug)]
pub struct DirEntry {
    /// Name as UTF-16 units, without the terminator.
    pub name: Vec<u16>,
    pub kind: u8,
    pub color: u8,
    pub left: u32,
    pub right: u32,
    pub child: u32,
    pub clsid: [u8; 16],
    pub state: u32,
    pub created: u64,
    pub modified: u64,
    pub start: u32,
    pub size: u64,
}

impl DirEntry {
    pub fn name_string(&self) -> String {
        String::from_utf16_lossy(&self.name)
    }
}

/// A parsed compound file over borrowed bytes.
pub struct Cfb<'a> {
    data: &'a [u8],
    sector_size: usize,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    pub entries: Vec<DirEntry>,
    mini_stream: Vec<u8>,
    /// `(path, entry index)` in directory-walk order, storages included.
    paths: Vec<(String, usize)>,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Case-insensitive CFB name comparison key (simple uppercase per unit).
fn upper(name: &str) -> String {
    name.chars().flat_map(char::to_uppercase).collect()
}

impl<'a> Cfb<'a> {
    pub fn is_cfb(data: &[u8]) -> bool {
        data.len() >= 8 && data[..4] == MAGIC[..4]
    }

    pub fn open(data: &'a [u8]) -> Result<Self, Error> {
        if data.len() < 512 || data[..8] != MAGIC {
            return Err(Error::Container("not a compound file".into()));
        }
        let shift = u16_at(data, 30);
        if !(9..=12).contains(&shift) {
            return Err(Error::Container(format!("bad sector shift {shift}")));
        }
        let sector_size = 1usize << shift;
        let physical = data.len().div_ceil(sector_size).saturating_sub(1);
        let sector = |s: u32| -> Option<&'a [u8]> {
            let s = s as usize;
            if s >= physical {
                return None;
            }
            let start = (s + 1) * sector_size;
            data.get(start..(start + sector_size).min(data.len()))
        };

        // FAT sector ids: 109 header slots, then the DIFAT chain.
        let declared_fat = u32_at(data, 44) as usize;
        let mut fat_ids = Vec::new();
        let mut seen = HashSet::new();
        let mut push_id = |id: u32, fat_ids: &mut Vec<u32>| {
            if id < physical as u32 && seen.insert(id) {
                fat_ids.push(id);
            }
        };
        for i in 0..109 {
            if fat_ids.len() >= declared_fat {
                break;
            }
            let id = u32_at(data, 76 + 4 * i);
            if id != FREE && id != END {
                push_id(id, &mut fat_ids);
            }
        }
        let mut difat = u32_at(data, 68);
        let difat_count = (u32_at(data, 72) as usize).min(MAX_DIFAT);
        let mut visited = HashSet::new();
        for _ in 0..difat_count {
            if fat_ids.len() >= declared_fat || difat == END || difat == FREE {
                break;
            }
            let Some(s) = sector(difat) else { break };
            if !visited.insert(difat) {
                break;
            }
            let per = sector_size / 4 - 1;
            for i in 0..per {
                if fat_ids.len() >= declared_fat {
                    break;
                }
                let id = u32_at(s, 4 * i);
                if id != FREE && id != END {
                    push_id(id, &mut fat_ids);
                }
            }
            difat = u32_at(s, sector_size - 4);
        }
        // Only the FAT sectors needed to index the file matter.
        let needed = physical.div_ceil(sector_size / 4).max(1);
        fat_ids.truncate(needed);
        let mut fat = Vec::with_capacity(fat_ids.len() * sector_size / 4);
        for &id in &fat_ids {
            let Some(s) = sector(id) else { continue };
            fat.extend(
                s.chunks_exact(4)
                    .map(|c| u32::from_le_bytes(c.try_into().unwrap())),
            );
        }

        let mut cfb = Cfb {
            data,
            sector_size,
            fat,
            mini_fat: Vec::new(),
            entries: Vec::new(),
            mini_stream: Vec::new(),
            paths: Vec::new(),
        };

        // Directory.
        let dir_bytes = cfb.chain(u32_at(data, 48), (MAX_ENTRIES * 128) as u64);
        for chunk in dir_bytes.chunks_exact(128).take(MAX_ENTRIES) {
            let name_len = (u16_at(chunk, 64) as usize).min(64);
            let units = name_len.saturating_sub(2) / 2;
            let name: Vec<u16> = (0..units).map(|i| u16_at(chunk, 2 * i)).collect();
            cfb.entries.push(DirEntry {
                name,
                kind: chunk[66],
                color: chunk[67],
                left: u32_at(chunk, 68),
                right: u32_at(chunk, 72),
                child: u32_at(chunk, 76),
                clsid: chunk[80..96].try_into().unwrap(),
                state: u32_at(chunk, 96),
                created: u64_at(chunk, 100),
                modified: u64_at(chunk, 108),
                start: u32_at(chunk, 116),
                size: u64_at(chunk, 120) & 0xFFFF_FFFF,
            });
        }
        if cfb.entries.first().is_none_or(|e| e.kind != ROOT) {
            return Err(Error::Container("missing root entry".into()));
        }
        let declared: u64 = cfb
            .entries
            .iter()
            .filter(|e| e.kind == STREAM)
            .map(|e| e.size)
            .sum();
        if declared > MAX_TOTAL {
            return Err(Error::Container("declared streams exceed 512 MiB".into()));
        }

        // Mini FAT and mini stream.
        let mini_fat_bytes = cfb.chain(u32_at(data, 60), MAX_TOTAL);
        cfb.mini_fat = mini_fat_bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let root = &cfb.entries[0];
        cfb.mini_stream = cfb.chain(root.start, root.size.min(MAX_TOTAL));
        cfb.walk();
        Ok(cfb)
    }

    /// Reads a regular sector chain, at most `limit` bytes, stopping on a
    /// revisit, an out-of-range sector or the end of chain.
    fn chain(&self, start: u32, limit: u64) -> Vec<u8> {
        let mut out = Vec::new();
        let mut s = start;
        let mut seen = HashSet::new();
        let physical = self.data.len().div_ceil(self.sector_size).saturating_sub(1);
        while (s as usize) < physical && (out.len() as u64) < limit && seen.insert(s) {
            let begin = (s as usize + 1) * self.sector_size;
            let end = (begin + self.sector_size).min(self.data.len());
            out.extend_from_slice(&self.data[begin..end]);
            s = self.fat.get(s as usize).copied().unwrap_or(END);
        }
        out.truncate(limit.min(out.len() as u64) as usize);
        out
    }

    fn mini_chain(&self, start: u32, size: u64) -> Vec<u8> {
        let mut out = Vec::with_capacity(size as usize);
        let mut s = start;
        let mut seen = HashSet::new();
        while (out.len() as u64) < size && seen.insert(s) {
            let begin = s as usize * MINI_SECTOR;
            let Some(chunk) = self
                .mini_stream
                .get(begin..(begin + MINI_SECTOR).min(self.mini_stream.len()))
            else {
                break;
            };
            out.extend_from_slice(chunk);
            s = self.mini_fat.get(s as usize).copied().unwrap_or(END);
        }
        out.truncate(size.min(out.len() as u64) as usize);
        out
    }

    /// Builds the path list by walking the red-black trees from the root.
    fn walk(&mut self) {
        let n = self.entries.len();
        let mut visited = vec![false; n];
        visited[0] = true;
        // (entry, parent path); siblings in left, self, right order.
        let mut out = Vec::new();
        self.walk_tree(
            self.entries[0].child,
            String::new(),
            &mut visited,
            &mut out,
            0,
        );
        self.paths = out;
    }

    fn walk_tree(
        &self,
        root: u32,
        parent: String,
        visited: &mut [bool],
        out: &mut Vec<(String, usize)>,
        depth: usize,
    ) {
        if depth > 64 {
            return;
        }
        // In-order traversal with an explicit stack.
        let mut stack = Vec::new();
        let mut cur = root;
        loop {
            while (cur as usize) < self.entries.len() && !visited[cur as usize] {
                visited[cur as usize] = true;
                stack.push(cur as usize);
                cur = self.entries[cur as usize].left;
            }
            let Some(i) = stack.pop() else { break };
            let e = &self.entries[i];
            let path = if parent.is_empty() {
                e.name_string()
            } else {
                format!("{parent}/{}", e.name_string())
            };
            if e.kind == STREAM || e.kind == STORAGE {
                out.push((path.clone(), i));
            }
            if e.kind == STORAGE {
                self.walk_tree(e.child, path, visited, out, depth + 1);
            }
            cur = e.right;
        }
    }

    /// Every stream and storage path, in directory-walk order.
    pub fn paths(&self) -> &[(String, usize)] {
        &self.paths
    }

    /// Finds an entry by `/`-separated path, case-insensitively.
    pub fn find(&self, path: &str) -> Option<usize> {
        let want = upper(path);
        self.paths
            .iter()
            .find(|(p, _)| upper(p) == want)
            .map(|&(_, i)| i)
    }

    pub fn exists(&self, path: &str) -> bool {
        self.find(path).is_some()
    }

    /// Reads a stream's bytes by entry index.
    pub fn read_entry(&self, i: usize) -> Vec<u8> {
        let e = &self.entries[i];
        if e.kind != STREAM {
            return Vec::new();
        }
        if e.size < MINI_CUTOFF {
            self.mini_chain(e.start, e.size)
        } else {
            self.chain(e.start, e.size)
        }
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.find(path).map(|i| self.read_entry(i))
    }
}

/// Writes a version 3 compound file.
///
/// `entries` is a directory whose tree links are already valid (usually the
/// source directory); `streams[i]` holds the content of entry `i` when it is
/// a stream. Start sectors and sizes are recomputed.
pub fn write(entries: &[DirEntry], streams: &[Option<&[u8]>]) -> Vec<u8> {
    const SS: usize = 512;
    let mut entries: Vec<DirEntry> = entries.to_vec();

    // Mini stream: streams under the cutoff, 64-byte aligned.
    let mut mini = Vec::new();
    let mut mini_fat: Vec<u32> = Vec::new();
    for (i, e) in entries.iter_mut().enumerate() {
        if e.kind != STREAM {
            continue;
        }
        let data = streams.get(i).copied().flatten().unwrap_or(&[]);
        e.size = data.len() as u64;
        if data.is_empty() {
            e.start = END;
            continue;
        }
        if (data.len() as u64) < MINI_CUTOFF {
            let first = (mini.len() / MINI_SECTOR) as u32;
            let count = data.len().div_ceil(MINI_SECTOR);
            mini.extend_from_slice(data);
            mini.resize(mini.len().next_multiple_of(MINI_SECTOR), 0);
            for k in 0..count as u32 {
                mini_fat.push(if k + 1 == count as u32 {
                    END
                } else {
                    first + k + 1
                });
            }
            e.start = first;
        }
    }

    // Regular sectors: big streams, mini stream, mini FAT, directory.
    let mut body: Vec<u8> = Vec::new();
    let mut fat: Vec<u32> = Vec::new();
    let place = |bytes: &[u8], body: &mut Vec<u8>, fat: &mut Vec<u32>| -> u32 {
        if bytes.is_empty() {
            return END;
        }
        let first = fat.len() as u32;
        let count = bytes.len().div_ceil(SS);
        body.extend_from_slice(bytes);
        body.resize(body.len().next_multiple_of(SS), 0);
        for k in 0..count as u32 {
            fat.push(if k + 1 == count as u32 {
                END
            } else {
                first + k + 1
            });
        }
        first
    };
    for (i, e) in entries.iter_mut().enumerate() {
        if e.kind != STREAM || e.size < MINI_CUTOFF {
            continue;
        }
        let data = streams.get(i).copied().flatten().unwrap_or(&[]);
        e.start = place(data, &mut body, &mut fat);
    }
    let mini_start = place(&mini, &mut body, &mut fat);
    entries[0].start = mini_start;
    entries[0].size = mini.len() as u64;
    let mini_fat_bytes: Vec<u8> = mini_fat.iter().flat_map(|v| v.to_le_bytes()).collect();
    let mini_fat_start = place(&mini_fat_bytes, &mut body, &mut fat);
    let mini_fat_sectors = mini_fat_bytes.len().div_ceil(SS) as u32;

    let mut dir = Vec::with_capacity(entries.len().next_multiple_of(4) * 128);
    for e in &entries {
        let mut rec = [0u8; 128];
        let units = e.name.len().min(31);
        for (k, u) in e.name[..units].iter().enumerate() {
            rec[2 * k..2 * k + 2].copy_from_slice(&u.to_le_bytes());
        }
        let name_len = if units == 0 && e.kind == 0 {
            0
        } else {
            (units as u16 + 1) * 2
        };
        rec[64..66].copy_from_slice(&name_len.to_le_bytes());
        rec[66] = e.kind;
        rec[67] = e.color;
        rec[68..72].copy_from_slice(&e.left.to_le_bytes());
        rec[72..76].copy_from_slice(&e.right.to_le_bytes());
        rec[76..80].copy_from_slice(&e.child.to_le_bytes());
        rec[80..96].copy_from_slice(&e.clsid);
        rec[96..100].copy_from_slice(&e.state.to_le_bytes());
        rec[100..108].copy_from_slice(&e.created.to_le_bytes());
        rec[108..116].copy_from_slice(&e.modified.to_le_bytes());
        let start = if e.kind == STORAGE || e.kind == 0 {
            0
        } else {
            e.start
        };
        rec[116..120].copy_from_slice(&start.to_le_bytes());
        rec[120..128].copy_from_slice(&e.size.to_le_bytes());
        dir.extend_from_slice(&rec);
    }
    while dir.len() % SS != 0 {
        let mut rec = [0u8; 128];
        rec[68..80].copy_from_slice(&[0xFF; 12]);
        dir.extend_from_slice(&rec);
    }
    let dir_start = place(&dir, &mut body, &mut fat);

    // FAT and DIFAT sectors cover every sector including themselves.
    let per = SS / 4;
    let data_sectors = fat.len();
    let (mut fat_n, mut difat_n) = (0usize, 0usize);
    loop {
        let total = data_sectors + fat_n + difat_n;
        let need_fat = total.div_ceil(per);
        let need_difat = need_fat.saturating_sub(109).div_ceil(per - 1);
        if need_fat == fat_n && need_difat == difat_n {
            break;
        }
        fat_n = need_fat;
        difat_n = need_difat;
    }
    let fat_first = data_sectors as u32;
    let difat_first = fat_first + fat_n as u32;
    fat.extend(std::iter::repeat_n(FAT_SECT, fat_n));
    fat.extend(std::iter::repeat_n(DIFAT_SECT, difat_n));
    fat.resize(fat_n * per, FREE);
    let fat_ids: Vec<u32> = (0..fat_n as u32).map(|k| fat_first + k).collect();

    let mut out = Vec::with_capacity(SS + body.len() + (fat_n + difat_n) * SS);
    let mut header = [0u8; 512];
    header[..8].copy_from_slice(&MAGIC);
    header[24..26].copy_from_slice(&0x003Eu16.to_le_bytes());
    header[26..28].copy_from_slice(&3u16.to_le_bytes());
    header[28..30].copy_from_slice(&0xFFFEu16.to_le_bytes());
    header[30..32].copy_from_slice(&9u16.to_le_bytes());
    header[32..34].copy_from_slice(&6u16.to_le_bytes());
    header[44..48].copy_from_slice(&(fat_n as u32).to_le_bytes());
    header[48..52].copy_from_slice(&dir_start.to_le_bytes());
    header[56..60].copy_from_slice(&(MINI_CUTOFF as u32).to_le_bytes());
    header[60..64].copy_from_slice(&mini_fat_start.to_le_bytes());
    header[64..68].copy_from_slice(&mini_fat_sectors.to_le_bytes());
    let first_difat = if difat_n > 0 { difat_first } else { END };
    header[68..72].copy_from_slice(&first_difat.to_le_bytes());
    header[72..76].copy_from_slice(&(difat_n as u32).to_le_bytes());
    for i in 0..109 {
        let id = fat_ids.get(i).copied().unwrap_or(FREE);
        header[76 + 4 * i..80 + 4 * i].copy_from_slice(&id.to_le_bytes());
    }
    out.extend_from_slice(&header);
    out.extend_from_slice(&body);
    for v in &fat {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let rest: Vec<u32> = fat_ids.iter().skip(109).copied().collect();
    for k in 0..difat_n {
        let mut sec = vec![FREE; per];
        for (j, slot) in sec.iter_mut().take(per - 1).enumerate() {
            if let Some(&id) = rest.get(k * (per - 1) + j) {
                *slot = id;
            }
        }
        sec[per - 1] = if k + 1 < difat_n {
            difat_first + k as u32 + 1
        } else {
            END
        };
        for v in sec {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// Order of sibling names in a directory: shorter first, then by upper case.
fn sibling_key(name: &str) -> (usize, String) {
    (name.encode_utf16().count(), upper(name))
}

/// A compound file holding `files` (paths with `/` between storage names,
/// e.g. `BinData/BIN0001.jpg`), storages made as their paths need them.
pub fn build(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    build_from(files, None)
}

/// Rebuilds a container with additional streams while retaining each existing
/// directory entry's metadata.
pub fn build_preserving(files: &[(String, Vec<u8>)], original: &Cfb<'_>) -> Vec<u8> {
    build_from(files, Some(original))
}

fn build_from(files: &[(String, Vec<u8>)], original: Option<&Cfb<'_>>) -> Vec<u8> {
    const NONE: u32 = u32::MAX;
    let entry = |name: &str, kind: u8| DirEntry {
        name: name.encode_utf16().collect(),
        kind,
        color: 1,
        left: NONE,
        right: NONE,
        child: NONE,
        clsid: [0; 16],
        state: 0,
        created: 0,
        modified: 0,
        start: 0,
        size: 0,
    };
    let named = |path: &str, name: &str, kind: u8| {
        let mut e = original
            .and_then(|c| {
                if path.is_empty() {
                    c.entries.first()
                } else {
                    c.find(path).map(|i| &c.entries[i])
                }
            })
            .cloned()
            .unwrap_or_else(|| entry(name, kind));
        e.left = NONE;
        e.right = NONE;
        e.child = NONE;
        e
    };
    let mut entries = vec![named("", "Root Entry", ROOT)];
    let mut streams: Vec<Option<&[u8]>> = vec![None];
    // Children of each entry by index, then linked as binary trees.
    let mut kids: Vec<Vec<usize>> = vec![Vec::new()];
    let mut storages: Vec<(String, usize)> = Vec::new();
    if let Some(original) = original {
        // Empty storages carry application metadata too. Seed them before
        // adding stream paths so rebuilding a container keeps them.
        for (path, index) in original.paths() {
            if original.entries[*index].kind != STORAGE {
                continue;
            }
            let mut parent = 0;
            let parts: Vec<_> = path.split('/').collect();
            for (k, part) in parts.iter().enumerate() {
                let key = parts[..=k].join("/");
                parent = match storages.iter().find(|s| s.0 == key) {
                    Some(&(_, i)) => i,
                    None => {
                        entries.push(named(&key, part, STORAGE));
                        streams.push(None);
                        kids.push(Vec::new());
                        let i = entries.len() - 1;
                        kids[parent].push(i);
                        storages.push((key, i));
                        i
                    }
                };
            }
        }
    }
    for (path, data) in files {
        let mut parent = 0;
        let parts: Vec<&str> = path.split('/').collect();
        for (k, part) in parts.iter().enumerate() {
            let last = k + 1 == parts.len();
            if last {
                entries.push(named(path, part, STREAM));
                streams.push(Some(data));
                kids.push(Vec::new());
                kids[parent].push(entries.len() - 1);
            } else {
                let key = parts[..=k].join("/");
                parent = match storages.iter().find(|s| s.0 == key) {
                    Some(&(_, i)) => i,
                    None => {
                        entries.push(named(&key, part, STORAGE));
                        streams.push(None);
                        kids.push(Vec::new());
                        let i = entries.len() - 1;
                        kids[parent].push(i);
                        storages.push((key, i));
                        i
                    }
                };
            }
        }
    }
    fn link(entries: &mut [DirEntry], ids: &[usize]) -> u32 {
        if ids.is_empty() {
            return u32::MAX;
        }
        let mid = ids.len() / 2;
        let (l, r) = (link(entries, &ids[..mid]), link(entries, &ids[mid + 1..]));
        let e = &mut entries[ids[mid]];
        e.left = l;
        e.right = r;
        ids[mid] as u32
    }
    for (parent, list) in kids.iter().enumerate() {
        let mut sorted = list.clone();
        sorted.sort_by_key(|&i| sibling_key(&entries[i].name_string()));
        let root = link(&mut entries, &sorted);
        entries[parent].child = root;
    }
    write(&entries, &streams)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: u32 = 0xFFFF_FFFF;

    fn entry(name: &str, kind: u8, left: u32, right: u32, child: u32) -> DirEntry {
        DirEntry {
            name: name.encode_utf16().collect(),
            kind,
            color: 1,
            left,
            right,
            child,
            clsid: [0; 16],
            state: 0,
            created: 0,
            modified: 0,
            start: 0,
            size: 0,
        }
    }

    #[test]
    fn written_files_read_back() {
        let entries = vec![
            entry("Root Entry", ROOT, NONE, NONE, 1),
            entry("Small", STREAM, NONE, 2, NONE),
            entry("Big", STREAM, NONE, 3, NONE),
            entry("Dir", STORAGE, NONE, NONE, 4),
            entry("Inner", STREAM, NONE, NONE, NONE),
        ];
        let small = b"hello".to_vec();
        let big: Vec<u8> = (0..20000u32).map(|i| (i % 251) as u8).collect();
        let inner: Vec<u8> = vec![7; 4096];
        let streams = vec![
            None,
            Some(&small[..]),
            Some(&big[..]),
            None,
            Some(&inner[..]),
        ];
        let bytes = write(&entries, &streams);
        let cfb = Cfb::open(&bytes).unwrap();
        assert_eq!(cfb.read("Small").unwrap(), small);
        assert_eq!(cfb.read("big").unwrap(), big);
        assert_eq!(cfb.read("Dir/Inner").unwrap(), inner);
        assert!(cfb.read("Dir/Missing").is_none());
    }

    #[test]
    fn adding_streams_keeps_empty_storages_and_directory_metadata() {
        let mut entries = vec![
            entry("Root Entry", ROOT, NONE, NONE, 1),
            entry("Empty", STORAGE, NONE, 2, NONE),
            entry("Existing", STREAM, NONE, NONE, NONE),
        ];
        entries[0].clsid = [7; 16];
        entries[1].state = 123;
        entries[1].created = 456;
        let bytes = write(&entries, &[None, None, Some(b"existing")]);
        let original = Cfb::open(&bytes).unwrap();
        let added = build_preserving(
            &[
                ("Existing".into(), b"existing".to_vec()),
                ("BinData/BIN0001.png".into(), b"image".to_vec()),
            ],
            &original,
        );
        let reopened = Cfb::open(&added).unwrap();
        let empty = &reopened.entries[reopened.find("Empty").unwrap()];
        assert_eq!(empty.kind, STORAGE);
        assert_eq!(empty.state, 123);
        assert_eq!(empty.created, 456);
        assert_eq!(reopened.entries[0].clsid, [7; 16]);
        assert_eq!(reopened.read("Existing").unwrap(), b"existing");
        assert_eq!(reopened.read("BinData/BIN0001.png").unwrap(), b"image");
    }

    #[test]
    fn many_sectors_need_difat() {
        let entries = vec![
            entry("Root Entry", ROOT, NONE, NONE, 1),
            entry("Huge", STREAM, NONE, NONE, NONE),
        ];
        let huge: Vec<u8> = (0..8_000_000u32).map(|i| (i * 7 % 256) as u8).collect();
        let bytes = write(&entries, &[None, Some(&huge[..])]);
        let cfb = Cfb::open(&bytes).unwrap();
        assert_eq!(cfb.read("Huge").unwrap(), huge);
    }
}
