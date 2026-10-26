//! The local CASC storage: `.build.info` names each product's build; its
//! build config the encoding table and the root; the local indexes
//! (`Data/data/*.idx`) where each encoded file is in the data archives.
//! One installation holds every product (retail, classic, the betas and
//! PTRs) in one store; a [`Casc`] reads one product's build of it.

use std::collections::HashMap;

use crate::db2::Db2;
use crate::{Storage, blte, jenkins};

#[derive(Debug)]
pub struct CascError(pub String);

impl std::fmt::Display for CascError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(s: impl Into<String>) -> Result<T, CascError> {
    Err(CascError(s.into()))
}

fn hex(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn to_hex(k: &[u8]) -> String {
    k.iter().map(|b| format!("{b:02x}")).collect()
}

/// One local index: its sorted entries (9-byte keys, archive, offset,
/// size), 18 bytes each.
struct Index {
    entries: Vec<u8>,
}

const ENTRY: usize = 18;

impl Index {
    fn parse(data: &[u8]) -> Option<Index> {
        if data.len() < 0x28 {
            return None;
        }
        let version = u16::from_le_bytes([data[8], data[9]]);
        let (size_len, offset_len, key_len) = (data[12], data[13], data[14]);
        if version != 7 || size_len != 4 || offset_len != 5 || key_len != 9 {
            return None;
        }
        let size = u32::from_le_bytes([data[0x20], data[0x21], data[0x22], data[0x23]]) as usize;
        let start = 0x28;
        let end = (start + size).min(data.len());
        let n = (end - start) / ENTRY;
        Some(Index {
            entries: data[start..start + n * ENTRY].to_vec(),
        })
    }

    fn key(&self, i: usize) -> &[u8] {
        &self.entries[i * ENTRY..i * ENTRY + 9]
    }

    /// Every place an encoded key is (the entries of a key lie together,
    /// the index being sorted): archive, offset, size.
    fn find_all(&self, ekey: &[u8]) -> Vec<(u32, u64, u32)> {
        let n = self.entries.len() / ENTRY;
        let probe = &ekey[..9];
        let Some(hit) = self.position(probe) else {
            return Vec::new();
        };
        let mut first = hit;
        while first > 0 && self.key(first - 1) == probe {
            first -= 1;
        }
        (first..n)
            .take_while(|&i| self.key(i) == probe)
            .map(|i| self.place(i))
            .collect()
    }

    /// An entry of a key: a binary search; a linear one if not sorted.
    fn position(&self, probe: &[u8]) -> Option<usize> {
        let n = self.entries.len() / ENTRY;
        let (mut lo, mut hi) = (0usize, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.key(mid).cmp(probe) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(mid),
            }
        }
        (0..n).find(|&i| self.key(i) == probe)
    }

    /// Entry `i`'s place: archive, offset, size.
    fn place(&self, i: usize) -> (u32, u64, u32) {
        let e = &self.entries[i * ENTRY..i * ENTRY + ENTRY];
        let packed = u64::from(e[9]) << 32
            | u64::from(e[10]) << 24
            | u64::from(e[11]) << 16
            | u64::from(e[12]) << 8
            | u64::from(e[13]);
        let size = u32::from_le_bytes([e[14], e[15], e[16], e[17]]);
        ((packed >> 30) as u32, packed & 0x3fff_ffff, size)
    }
}

/// The bucket a key's index is in.
fn bucket(ekey: &[u8]) -> usize {
    let x = ekey[..9].iter().fold(0u8, |a, b| a ^ b);
    ((x & 0xf) ^ (x >> 4)) as usize
}

/// The encoding table: content keys to encoded keys.
struct Encoding {
    data: Vec<u8>,
    ckey_size: usize,
    ekey_size: usize,
    page_size: usize,
    /// Each page's first content key and where it starts.
    pages: Vec<([u8; 16], usize)>,
}

impl Encoding {
    fn parse(data: Vec<u8>) -> Result<Encoding, CascError> {
        if data.len() < 22 || &data[..2] != b"EN" {
            return err("the encoding table is not one");
        }
        let ckey_size = data[3] as usize;
        let ekey_size = data[4] as usize;
        let page_kb = u16::from_be_bytes([data[5], data[6]]) as usize;
        let page_count = u32::from_be_bytes([data[9], data[10], data[11], data[12]]) as usize;
        let espec_size = u32::from_be_bytes([data[18], data[19], data[20], data[21]]) as usize;
        let index_start = 22 + espec_size;
        let pages_start = index_start + page_count * (ckey_size + 16);
        let page_size = page_kb * 1024;
        let mut pages = Vec::with_capacity(page_count);
        for i in 0..page_count {
            let at = index_start + i * (ckey_size + 16);
            if at + 16 > data.len() {
                return err("a short encoding table");
            }
            let mut first = [0u8; 16];
            first.copy_from_slice(&data[at..at + 16]);
            pages.push((first, pages_start + i * page_size));
        }
        Ok(Encoding {
            data,
            ckey_size,
            ekey_size,
            page_size,
            pages,
        })
    }

    /// The encoded keys of a content key (most have one; some have several,
    /// and the local storage may hold any of them).
    fn ekeys(&self, ckey: &[u8; 16]) -> Vec<[u8; 16]> {
        let i = match self.pages.binary_search_by(|(first, _)| first.cmp(ckey)) {
            Ok(i) => i,
            Err(0) => return Vec::new(),
            Err(i) => i - 1,
        };
        let start = self.pages[i].1;
        let end = (start + self.page_size).min(self.data.len());
        let mut pos = start;
        while pos + 6 + self.ckey_size <= end {
            let count = self.data[pos] as usize;
            if count == 0 {
                break;
            }
            let ck = &self.data[pos + 6..pos + 6 + self.ckey_size];
            if ck == ckey {
                let n = self.ekey_size.min(16);
                return (0..count)
                    .filter_map(|k| {
                        let e = pos + 6 + self.ckey_size + k * self.ekey_size;
                        let mut out = [0u8; 16];
                        out[..n].copy_from_slice(self.data.get(e..e + n)?);
                        Some(out)
                    })
                    .collect();
            }
            pos += 6 + self.ckey_size + count * self.ekey_size;
        }
        Vec::new()
    }

    fn ekey(&self, ckey: &[u8; 16]) -> Option<[u8; 16]> {
        self.ekeys(ckey).into_iter().next()
    }
}

/// The root: file IDs to content keys, name hashes to file IDs.
struct Root {
    by_id: Vec<(u32, [u8; 16])>,
    by_name: Vec<(u64, u32)>,
}

/// Locales a file may be for: enUS (2) or all.
const ENUS: u32 = 0x2;
/// Content flags of files the client does not use here.
const LOW_VIOLENCE: u32 = 0x80;
const NO_NAME_HASH: u32 = 0x1000_0000;

impl Root {
    fn parse(d: &[u8]) -> Result<Root, CascError> {
        let le = |at: usize| -> Option<u32> {
            d.get(at..at + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let mut pos = 0;
        let mut version = 0;
        if d.starts_with(b"TSFM") {
            // 8.2 on: a header; 10.1.7 on, its size and a version.
            let second = le(4).unwrap_or(0);
            if second == 0x18 {
                version = le(8).unwrap_or(0);
                pos = 0x18;
            } else {
                pos = 12;
            }
        }
        let mut by_id = Vec::new();
        let mut by_name = Vec::new();
        while pos + 12 <= d.len() {
            let count = le(pos).ok_or(CascError("root".into()))? as usize;
            let (content, locale);
            if version >= 2 {
                locale = le(pos + 4).unwrap_or(0);
                let f1 = le(pos + 8).unwrap_or(0);
                let f2 = le(pos + 12).unwrap_or(0);
                let f3 = u32::from(*d.get(pos + 16).unwrap_or(&0));
                content = f1 | f2 | (f3 << 17);
                pos += 17;
            } else {
                content = le(pos + 4).unwrap_or(0);
                locale = le(pos + 8).unwrap_or(0);
                pos += 12;
            }
            let named = !(content & NO_NAME_HASH != 0
                || (d.starts_with(b"TSFM") && version == 0 && content & NO_NAME_HASH != 0));
            let ids_at = pos;
            let keys_at = ids_at + count * 4;
            let names_at = keys_at + count * 16;
            let end = if named {
                names_at + count * 8
            } else {
                names_at
            };
            if end > d.len() {
                return err("a short root");
            }
            // enUS (or every locale) first, then other locales, the
            // low-violence files last: a file only listed for another
            // locale is still found (classic's tables).
            let rank: u8 = if content & LOW_VIOLENCE != 0 {
                2
            } else if locale & ENUS != 0 || locale == 0xffff_ffff {
                0
            } else {
                1
            };
            {
                let mut id: i64 = -1;
                for i in 0..count {
                    let delta = i32::from_le_bytes(
                        d[ids_at + i * 4..ids_at + i * 4 + 4]
                            .try_into()
                            .unwrap_or([0; 4]),
                    );
                    id += 1 + i64::from(delta);
                    let mut ck = [0u8; 16];
                    ck.copy_from_slice(&d[keys_at + i * 16..keys_at + i * 16 + 16]);
                    by_id.push((id as u32, rank, ck));
                    if named {
                        let h = u64::from_le_bytes(
                            d[names_at + i * 8..names_at + i * 8 + 8]
                                .try_into()
                                .unwrap_or([0; 8]),
                        );
                        by_name.push((h, rank, id as u32));
                    }
                }
            }
            pos = end;
        }
        // Every entry of an ID, the best first: one may be missing from
        // the local storage or encrypted with a key the client lacks.
        by_id.sort_by_key(|e| (e.0, e.1));
        by_id.dedup_by_key(|e| (e.0, e.2));
        by_name.sort_by_key(|e| (e.0, e.1));
        by_name.dedup_by_key(|e| e.0);
        Ok(Root {
            by_id: by_id.into_iter().map(|(id, _, ck)| (id, ck)).collect(),
            by_name: by_name.into_iter().map(|(h, _, id)| (h, id)).collect(),
        })
    }

    fn ckey(&self, id: u32) -> Option<[u8; 16]> {
        self.ckeys(id).first().map(|e| e.1)
    }

    /// An ID's entries, the best first.
    fn ckeys(&self, id: u32) -> &[(u32, [u8; 16])] {
        let start = self.by_id.partition_point(|e| e.0 < id);
        let end = self.by_id.partition_point(|e| e.0 <= id);
        &self.by_id[start..end]
    }

    fn id_of(&self, hash: u64) -> Option<u32> {
        self.by_name
            .binary_search_by_key(&hash, |e| e.0)
            .ok()
            .map(|i| self.by_name[i].1)
    }
}

/// One product's build of the storage.
pub struct Casc {
    storage: Box<dyn Storage + Send + Sync>,
    indexes: Vec<Option<Index>>,
    encoding: Encoding,
    root: Root,
    pub product: String,
    pub version: String,
    /// The client's encryption keys: its own tables' and those its hotfix
    /// cache holds (decision T317).
    keys: blte::Keys,
    /// The hotfix cache's records by table hash, read on first use.
    hotfixes: std::sync::OnceLock<HashMap<u32, Vec<Hotfix>>>,
    /// Game data `wowlua data pull` copied (see [`Casc::pull`]): files are
    /// read decoded from `files/<id>`.
    pulled: bool,
}

/// The client folder of pulled game data (its `Cache/ADB` and `Cache/WDB`).
const PULLED_FOLDER: &str = "_pulled_";

/// A hotfix of a table's record: its ID, its status (1: the record as
/// given, 2: the record removed) and its data.
pub type Hotfix = (u32, u8, Vec<u8>);

impl Casc {
    /// The products `.build.info` lists: (product, version).
    pub fn products(storage: &dyn Storage) -> Vec<(String, String)> {
        build_rows(storage)
            .into_iter()
            .map(|r| (r.product, r.version))
            .collect()
    }

    /// The product of a client folder (`_classic_beta_` →
    /// `wow_classic_beta`), from its `.flavor.info`.
    pub fn product_of_folder(storage: &dyn Storage, folder: &str) -> Option<String> {
        let text = storage.read(&format!("{folder}/.flavor.info"))?;
        let text = String::from_utf8_lossy(&text).into_owned();
        text.lines()
            .skip(1)
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_string)
    }

    /// Opens `product`'s build (`wow`, `wow_classic_beta`, …).
    pub fn open(storage: Box<dyn Storage + Send + Sync>, product: &str) -> Result<Casc, CascError> {
        let rows = build_rows(&*storage);
        let Some(row) = rows.iter().find(|r| r.product == product && r.active) else {
            return err(format!("the installation has no build of {product}"));
        };
        let k = &row.build_key;
        let config = storage
            .read(&format!("Data/config/{}/{}/{k}", &k[0..2], &k[2..4]))
            .ok_or_else(|| CascError(format!("no build config {k}")))?;
        let config = String::from_utf8_lossy(&config).into_owned();
        let field = |name: &str| -> Vec<String> {
            config
                .lines()
                .find_map(|l| {
                    let (key, v) = l.split_once('=')?;
                    (key.trim() == name).then(|| v.split_whitespace().map(str::to_string).collect())
                })
                .unwrap_or_default()
        };
        let root_ckey = field("root")
            .first()
            .and_then(|h| hex(h))
            .ok_or(CascError("no root".into()))?;
        let encoding_ekey = field("encoding")
            .get(1)
            .and_then(|h| hex(h))
            .ok_or(CascError("no encoding".into()))?;
        // The newest index of each bucket.
        let mut newest: Vec<Option<(u32, String)>> = vec![None; 16];
        for name in storage.list("Data/data") {
            let Some(stem) = name.strip_suffix(".idx") else {
                continue;
            };
            if stem.len() != 10 {
                continue;
            }
            let (Ok(b), Ok(v)) = (
                u8::from_str_radix(&stem[..2], 16),
                u32::from_str_radix(&stem[2..], 16),
            ) else {
                continue;
            };
            let slot = &mut newest[(b & 0xf) as usize];
            if slot.as_ref().is_none_or(|(old, _)| v > *old) {
                *slot = Some((v, name));
            }
        }
        let indexes = newest
            .into_iter()
            .map(|n| {
                n.and_then(|(_, name)| {
                    storage
                        .read(&format!("Data/data/{name}"))
                        .and_then(|d| Index::parse(&d))
                })
            })
            .collect();
        let mut casc = Casc {
            storage,
            indexes,
            encoding: Encoding {
                data: Vec::new(),
                ckey_size: 16,
                ekey_size: 16,
                page_size: 0,
                pages: Vec::new(),
            },
            root: Root {
                by_id: Vec::new(),
                by_name: Vec::new(),
            },
            product: product.to_string(),
            version: row.version.clone(),
            keys: blte::Keys::new(),
            hotfixes: std::sync::OnceLock::new(),
            pulled: false,
        };
        let enc = casc.read_ekey(&encoding_ekey).ok_or_else(|| {
            CascError(format!(
                "the encoding table {} is not in the local storage",
                to_hex(&encoding_ekey)
            ))
        })?;
        casc.encoding = Encoding::parse(enc)?;
        let root = casc
            .read_ckey(&root_ckey)
            .ok_or_else(|| CascError("the root file is not in the local storage".into()))?;
        casc.root = Root::parse(&root)?;
        casc.keys = casc.read_keys();
        Ok(casc)
    }

    /// The keys of the client's `TactKeyLookup` and `TactKey` tables,
    /// then those of its hotfix caches (`Cache/ADB/<locale>/DBCache.bin`).
    fn read_keys(&self) -> blte::Keys {
        const LOOKUP: u32 = 1302851;
        const KEY: u32 = 1302850;
        const LOOKUP_HASH: u32 = 0xAFC1_90D1;
        const KEY_HASH: u32 = 0xDF2F_53CF;
        let mut names: HashMap<u32, u64> = HashMap::new();
        let mut values: HashMap<u32, [u8; 16]> = HashMap::new();
        let bytes = |v: Vec<u32>| v.into_iter().map(|b| b as u8).collect::<Vec<u8>>();
        if let Some(db) = self.file_by_id(LOOKUP).and_then(Db2::parse) {
            for i in 0..db.len() {
                if let Ok(name) = <[u8; 8]>::try_from(bytes(db.ints(i, 0)).as_slice()) {
                    names.insert(db.id(i), u64::from_le_bytes(name));
                }
            }
        }
        if let Some(db) = self.file_by_id(KEY).and_then(Db2::parse) {
            for i in 0..db.len() {
                if let Ok(key) = <[u8; 16]>::try_from(bytes(db.ints(i, 0)).as_slice()) {
                    values.insert(db.id(i), key);
                }
            }
        }
        for folder in self.folders() {
            let adb = format!("{folder}/Cache/ADB");
            for locale in self.storage.list(&adb) {
                let Some(cache) = self.storage.read(&format!("{adb}/{locale}/DBCache.bin")) else {
                    continue;
                };
                for (table, record, data) in hotfixes(&cache) {
                    if table == LOOKUP_HASH && data.len() == 8 {
                        names.insert(record, u64::from_le_bytes(data.try_into().unwrap()));
                    } else if table == KEY_HASH && data.len() == 16 {
                        values.insert(record, data.try_into().unwrap());
                    }
                }
            }
        }
        names
            .into_iter()
            .filter_map(|(id, name)| Some((name, *values.get(&id)?)))
            .collect()
    }

    /// The hotfixes the server sent the client for a table (by its
    /// `Db2::table_hash`), from its hotfix cache
    /// (`Cache/ADB/<locale>/DBCache.bin`, enUS first), in their order: a
    /// later one of a record wins (decision T391).
    pub fn hotfixes(&self, table_hash: u32) -> &[Hotfix] {
        let all = self.hotfixes.get_or_init(|| {
            let mut by_table: HashMap<u32, Vec<Hotfix>> = HashMap::new();
            for folder in self.folders() {
                let adb = format!("{folder}/Cache/ADB");
                let mut locales = self.storage.list(&adb);
                locales.sort_by_key(|l| l != "enUS");
                let Some(cache) = locales
                    .iter()
                    .find_map(|l| self.storage.read(&format!("{adb}/{l}/DBCache.bin")))
                else {
                    continue;
                };
                for (table, record, status, data) in hotfix_records(&cache) {
                    by_table
                        .entry(table)
                        .or_default()
                        .push((record, status, data.to_vec()));
                }
            }
            by_table
        });
        all.get(&table_hash).map_or(&[], Vec::as_slice)
    }

    /// The installation's client folders of this product (`_classic_beta_`).
    fn folders(&self) -> Vec<String> {
        self.storage
            .list("")
            .into_iter()
            .filter(|f| f.starts_with('_') && f.ends_with('_'))
            .filter(|f| {
                Self::product_of_folder(&*self.storage, f).as_deref() == Some(&self.product)
            })
            .collect()
    }

    /// A cache file of the product's client (`Cache/WDB/<locale>/<name>`),
    /// each locale's.
    pub fn cache_files(&self, name: &str) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for folder in self.folders() {
            let wdb = format!("{folder}/Cache/WDB");
            for locale in self.storage.list(&wdb) {
                if let Some(b) = self.storage.read(&format!("{wdb}/{locale}/{name}")) {
                    out.push(b);
                }
            }
        }
        out
    }

    /// The number of encryption keys known.
    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    /// A file by its encoded key, decoded.
    pub fn read_ekey(&self, ekey: &[u8; 16]) -> Option<Vec<u8>> {
        let index = self.indexes.get(bucket(ekey))?.as_ref()?;
        // Every entry of the key, as the index may hold more than one (a
        // newer client writes a second pointing past the block's header).
        index
            .find_all(ekey)
            .into_iter()
            .find_map(|place| self.decode_at(place).ok())
    }

    /// The file at a place of the data archives, decoded: its BLTE data
    /// after a 30-byte header, or at once.
    fn decode_at(&self, (archive, offset, size): (u32, u64, u32)) -> Result<Vec<u8>, String> {
        let raw = self
            .storage
            .read_at(
                &format!("Data/data/data.{archive:03}"),
                offset,
                size as usize,
            )
            .ok_or("unreadable")?;
        let blte = if raw.starts_with(b"BLTE") {
            &raw[..]
        } else {
            raw.get(30..).ok_or("short")?
        };
        blte::decode(blte, &self.keys).map_err(|e| e.to_string())
    }

    pub fn read_ckey(&self, ckey: &[u8; 16]) -> Option<Vec<u8>> {
        self.encoding
            .ekeys(ckey)
            .iter()
            .find_map(|e| self.read_ekey(e))
    }

    /// A file by its ID.
    pub fn file_by_id(&self, id: u32) -> Option<Vec<u8>> {
        if self.pulled {
            return self.storage.read(&format!("files/{id}"));
        }
        self.root
            .ckeys(id)
            .iter()
            .find_map(|(_, ck)| self.read_ckey(ck))
    }

    /// A file's ID by its path (`Interface\Buttons\WHITE8X8.blp`).
    pub fn id_of(&self, path: &str) -> Option<u32> {
        self.root.id_of(jenkins::name_hash(path))
    }

    /// A file by its path; a texture path without an extension finds its
    /// `.blp`, and so does one named `.tga` or `.png` the storage lacks (the
    /// client takes `Interface\\FriendsFrame\\StatusIcon-Online.tga` from
    /// its `.blp`: Blizzard's friends status icons).
    pub fn file_by_path(&self, path: &str) -> Option<Vec<u8>> {
        self.file_by_id(self.id_of_texture(path)?)
    }

    /// The ID of a file by its path, as [`Casc::file_by_path`] finds it.
    pub fn id_of_texture(&self, path: &str) -> Option<u32> {
        self.id_of(path).or_else(|| {
            let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
            match name.rsplit_once('.') {
                None => self.id_of(&format!("{path}.blp")),
                Some((_, ext))
                    if ext.eq_ignore_ascii_case("tga") || ext.eq_ignore_ascii_case("png") =>
                {
                    let stem = &path[..path.len() - ext.len() - 1];
                    self.id_of(&format!("{stem}.blp"))
                }
                Some(_) => None,
            }
        })
    }

    /// Whether the root lists a file ID (its data may still be missing
    /// from the local storage).
    pub fn has_id(&self, id: u32) -> bool {
        self.root.ckey(id).is_some()
    }

    /// Whether the encoding table knows a file's content key.
    pub fn has_encoding(&self, id: u32) -> bool {
        self.root
            .ckey(id)
            .and_then(|c| self.encoding.ekey(&c))
            .is_some()
    }

    /// What happens reading a file ID, step by step (for reports).
    pub fn explain(&self, id: u32) -> String {
        if self.root.ckeys(id).is_empty() {
            return "not in the root".into();
        }
        let mut out = String::new();
        for (_, ck) in self.root.ckeys(id) {
            out.push_str(&format!("content key {}\n", to_hex(ck)));
            for e in self.encoding.ekeys(ck) {
                out.push_str(&format!("\n  encoded key {}", to_hex(&e)));
                let Some(index) = self.indexes.get(bucket(&e)).and_then(Option::as_ref) else {
                    out.push_str(": no index for its bucket");
                    continue;
                };
                let places = index.find_all(&e);
                if places.is_empty() {
                    out.push_str(": not in the local storage");
                    continue;
                }
                for place @ (archive, offset, size) in places {
                    out.push_str(&format!(": data.{archive:03} at {offset}, {size} bytes"));
                    match self.decode_at(place) {
                        Ok(d) => out.push_str(&format!(", {} bytes decoded", d.len())),
                        Err(e) => out.push_str(&format!(", {e}")),
                    }
                }
            }
            out.push('\n');
        }
        out
    }

    /// Files the root knows.
    pub fn file_count(&self) -> usize {
        self.root.by_id.len()
    }

    /// The game data of the build (`wowlua data pull`, decision T203): the
    /// client tables the engine reads, the files of `paths` (fonts), and
    /// the client's hotfix and WDB caches, as files relative to a folder
    /// that [`Casc::open_pulled`] reads back. Everything is decoded.
    pub fn pull(&self, paths: &[&str]) -> Vec<(String, Vec<u8>)> {
        let mut out = vec![(
            ".pulled".to_string(),
            format!("{}\n{}\n", self.product, self.version).into_bytes(),
        )];
        out.push((
            format!("{PULLED_FOLDER}/.flavor.info"),
            format!("Product\n{}\n", self.product).into_bytes(),
        ));
        let mut ids: Vec<u32> = crate::tables::file_ids()
            .chain(crate::atlas::file_ids())
            .collect();
        let mut names = String::new();
        for path in paths {
            if let Some(id) = self.id_of(path) {
                names.push_str(&format!("{:016x}\t{id}\n", jenkins::name_hash(path)));
                ids.push(id);
            }
        }
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            if let Some(bytes) = self.file_by_id(id) {
                out.push((format!("files/{id}"), bytes));
            }
        }
        out.push(("names".to_string(), names.into_bytes()));
        let mut seen = std::collections::HashSet::new();
        for folder in self.folders() {
            for kind in ["ADB", "WDB"] {
                let dir = format!("{folder}/Cache/{kind}");
                for locale in self.storage.list(&dir) {
                    for name in self.storage.list(&format!("{dir}/{locale}")) {
                        let rel = format!("Cache/{kind}/{locale}/{name}");
                        if !seen.insert(rel.clone()) {
                            continue;
                        }
                        if let Some(bytes) = self.storage.read(&format!("{folder}/{rel}")) {
                            out.push((format!("{PULLED_FOLDER}/{rel}"), bytes));
                        }
                    }
                }
            }
        }
        out
    }

    /// Game data [`Casc::pull`] wrote to `storage`: the same product and
    /// version, its tables and named files, and its caches.
    pub fn open_pulled(storage: Box<dyn Storage + Send + Sync>) -> Result<Casc, CascError> {
        let info = storage
            .read(".pulled")
            .ok_or_else(|| CascError("no pulled game data".into()))?;
        let info = String::from_utf8_lossy(&info).into_owned();
        let mut lines = info.lines();
        let product = lines.next().unwrap_or_default().to_string();
        let version = lines.next().unwrap_or_default().to_string();
        let names = storage.read("names").unwrap_or_default();
        let mut by_name: Vec<(u64, u32)> = String::from_utf8_lossy(&names)
            .lines()
            .filter_map(|l| {
                let (hash, id) = l.split_once('\t')?;
                Some((u64::from_str_radix(hash, 16).ok()?, id.parse().ok()?))
            })
            .collect();
        by_name.sort_unstable();
        by_name.dedup_by_key(|e| e.0);
        Ok(Casc {
            storage,
            indexes: Vec::new(),
            encoding: Encoding {
                data: Vec::new(),
                ckey_size: 16,
                ekey_size: 16,
                page_size: 0,
                pages: Vec::new(),
            },
            root: Root {
                by_id: Vec::new(),
                by_name,
            },
            product,
            version,
            keys: blte::Keys::new(),
            hotfixes: std::sync::OnceLock::new(),
            pulled: true,
        })
    }
}

/// A hotfix cache's valid records: (table hash, record ID, data).
fn hotfixes(cache: &[u8]) -> Vec<(u32, u32, &[u8])> {
    hotfix_records(cache)
        .into_iter()
        .filter(|r| r.2 == 1)
        .map(|(table, record, _, data)| (table, record, data))
        .collect()
}

/// A hotfix cache's records: (table hash, record ID, status, data).
fn hotfix_records(cache: &[u8]) -> Vec<(u32, u32, u8, &[u8])> {
    let u = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(cache.get(at..at + 4)?.try_into().ok()?))
    };
    let mut out = Vec::new();
    if cache.get(..4) != Some(b"XFTH") || u(4).is_none_or(|v| v < 8) {
        return out;
    }
    let mut pos = 44;
    while cache.get(pos..pos + 4) == Some(b"XFTH") {
        let (Some(table), Some(record), Some(size)) = (u(pos + 16), u(pos + 20), u(pos + 24))
        else {
            break;
        };
        let status = cache.get(pos + 28).copied().unwrap_or(0);
        let Some(data) = cache.get(pos + 32..pos + 32 + size as usize) else {
            break;
        };
        out.push((table, record, status, data));
        pos += 32 + size as usize;
    }
    out
}

struct BuildRow {
    product: String,
    version: String,
    build_key: String,
    active: bool,
}

fn build_rows(storage: &dyn Storage) -> Vec<BuildRow> {
    let Some(text) = storage.read(".build.info") else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&text).into_owned();
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return Vec::new();
    };
    let cols: Vec<String> = header
        .split('|')
        .map(|c| c.split('!').next().unwrap_or("").to_string())
        .collect();
    let at = |name: &str| cols.iter().position(|c| c == name);
    let (Some(p), Some(v), Some(k), a) =
        (at("Product"), at("Version"), at("Build Key"), at("Active"))
    else {
        return Vec::new();
    };
    lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split('|').collect();
            let get = |i: usize| f.get(i).copied().unwrap_or("").to_string();
            BuildRow {
                product: get(p),
                version: get(v),
                build_key: get(k),
                active: a.is_none_or(|a| get(a) == "1"),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An index file (version 7, 9-byte keys) of entries: key, archive,
    /// offset, size.
    fn index(entries: &[([u8; 9], u32, u64, u32)]) -> Vec<u8> {
        let mut d = vec![0u8; 0x28];
        d[0] = 16;
        d[8..10].copy_from_slice(&7u16.to_le_bytes());
        d[12] = 4;
        d[13] = 5;
        d[14] = 9;
        let body: Vec<u8> = entries
            .iter()
            .flat_map(|(k, a, o, s)| {
                let packed = (u64::from(*a) << 30) | o;
                let mut e = k.to_vec();
                e.extend(&packed.to_be_bytes()[3..]);
                e.extend(s.to_le_bytes());
                e
            })
            .collect();
        d[0x20..0x24].copy_from_slice(&(body.len() as u32).to_le_bytes());
        d.extend(body);
        d
    }

    /// A key's every entry, in order (a newer client writes a second, past
    /// the block's header); none for a key not there.
    #[test]
    fn an_index_gives_every_entry_of_a_key() {
        let a = [1u8; 9];
        let b = [2u8; 9];
        let c = [3u8; 9];
        let idx = Index::parse(&index(&[
            (a, 1, 10, 100),
            (b, 75, 825_339_748, 141_273_066),
            (b, 75, 825_339_778, 141_273_036),
            (c, 2, 0, 5),
        ]))
        .expect("parses");
        let key = |k: [u8; 9]| {
            let mut e = [0u8; 16];
            e[..9].copy_from_slice(&k);
            e
        };
        assert_eq!(
            idx.find_all(&key(b)),
            [
                (75, 825_339_748, 141_273_066),
                (75, 825_339_778, 141_273_036)
            ]
        );
        assert_eq!(idx.find_all(&key(a)), [(1, 10, 100)]);
        assert!(idx.find_all(&key([9u8; 9])).is_empty());
    }
}
