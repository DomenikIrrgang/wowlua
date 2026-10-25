//! The client's tables (DB2, the WDC4 and WDC5 formats): records of
//! fields, each stored inline, bit-packed, in a palette or as a default
//! with exceptions. Read by field index; strings by their offsets into the
//! sections' string tables. Sparse tables (an offset map: `ItemSparse`)
//! have records of their own sizes, their fields one after another and
//! their strings inline, so which fields are strings must be told
//! ([`Db2::set_string_fields`]).

/// How a field is stored.
#[derive(Clone, Copy, Debug)]
struct Field {
    offset_bits: u32,
    size_bits: u32,
    storage: u32,
    /// Common data: the default; pallets: the array's length.
    val1: u32,
    val3: u32,
    /// Where its palette or common data starts in that block.
    extra: usize,
    extra_len: usize,
    /// Inline fields: the byte offset and size.
    byte_offset: usize,
    byte_size: usize,
}

/// A record: where it is in the file, its virtual position (for strings),
/// its ID.
#[derive(Clone, Copy, Debug)]
struct Record {
    at: usize,
    virtual_at: usize,
    id: u32,
    /// The parent record's ID a relationship map gives (a field not in
    /// the record), else 0.
    relation: u32,
    /// A sparse record's size.
    size: usize,
    /// A hotfix's record: in `Db2::hot`, its fields one after another at
    /// their full sizes, its strings inline.
    hot: bool,
}

pub struct Db2 {
    data: Vec<u8>,
    fields: Vec<Field>,
    records: Vec<Record>,
    pallet: (usize, usize),
    common: (usize, usize),
    /// The string tables of all sections, in order: (file offset, size).
    strings: Vec<(usize, usize)>,
    /// All sections' records' bytes (strings' virtual offsets start there).
    records_bytes: usize,
    pub layout_hash: u32,
    /// The table's hash, by which hotfixes name it.
    pub table_hash: u32,
    /// The hotfixes' records' bytes.
    hot: Vec<u8>,
    /// Each field's size in a hotfix record (bytes per element).
    hot_sizes: Vec<usize>,
    /// Each field's size as the layout gives it, where known: a value of
    /// a palette or the common data is cut to it (its high bytes are not
    /// the field's).
    natural: Vec<Option<usize>>,
    by_id: std::sync::OnceLock<std::collections::HashMap<u32, usize>>,
    /// Records of their own sizes (an offset map).
    sparse: bool,
    /// A sparse table's string fields, by index.
    string_fields: Vec<bool>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

impl Db2 {
    pub fn parse(data: Vec<u8>) -> Option<Db2> {
        let h = match data.get(..4)? {
            b"WDC5" => 136,
            b"WDC4" | b"WDC3" => 4,
            _ => return None,
        };
        let u = |i: usize| u32_at(&data, h + i * 4);
        let record_count = u(0)? as usize;
        let field_count = u(1)? as usize;
        let record_size = u(2)? as usize;
        let table_hash = u(4)?;
        let layout_hash = u(5)?;
        let flags = u16_at(&data, h + 36)?;
        let id_index = u16_at(&data, h + 38)? as usize;
        let total_fields = u32_at(&data, h + 40)? as usize;
        let fsi_size = u32_at(&data, h + 52)? as usize;
        let common_size = u32_at(&data, h + 56)? as usize;
        let pallet_size = u32_at(&data, h + 60)? as usize;
        let section_count = u32_at(&data, h + 64)? as usize;
        let _ = (record_count, field_count);
        let sparse = flags & 1 != 0;
        let sections_at = h + 68;
        let structs_at = sections_at + section_count * 40;
        let fsi_at = structs_at + total_fields * 4;
        let pallet_at = fsi_at + fsi_size;
        let common_at = pallet_at + pallet_size;
        let mut fields = Vec::new();
        let (mut pallet_off, mut common_off) = (0usize, 0usize);
        for i in 0..fsi_size / 24 {
            let f = fsi_at + i * 24;
            let offset_bits = u32::from(u16_at(&data, f)?);
            let size_bits = u32::from(u16_at(&data, f + 2)?);
            let extra_len = u32_at(&data, f + 4)? as usize;
            let storage = u32_at(&data, f + 8)?;
            let val1 = u32_at(&data, f + 12)?;
            let val3 = u32_at(&data, f + 20)?;
            let extra = match storage {
                2 => {
                    let o = common_off;
                    common_off += extra_len;
                    o
                }
                3 | 4 => {
                    let o = pallet_off;
                    pallet_off += extra_len;
                    o
                }
                _ => 0,
            };
            let st = structs_at + i * 4;
            let size = i16::from_le_bytes(data.get(st..st + 2)?.try_into().ok()?);
            let byte_offset = u16_at(&data, st + 2)? as usize;
            fields.push(Field {
                offset_bits,
                size_bits,
                storage,
                val1,
                val3,
                extra,
                extra_len,
                byte_offset,
                byte_size: ((32 - i32::from(size)) / 8).max(0) as usize,
            });
        }
        let mut records = Vec::new();
        let mut strings = Vec::new();
        let mut copies: Vec<(u32, u32)> = Vec::new();
        // Records whose ID is the `id_index` field (no ID list).
        let mut id_from_field: Vec<usize> = Vec::new();
        let mut virtual_at = 0usize;
        for s in 0..section_count {
            let sh = sections_at + s * 40;
            let tact_key = u64::from_le_bytes(data.get(sh..sh + 8)?.try_into().ok()?);
            let file_offset = u32_at(&data, sh + 8)? as usize;
            let count = u32_at(&data, sh + 12)? as usize;
            let string_size = u32_at(&data, sh + 16)? as usize;
            let id_list_size = u32_at(&data, sh + 24)? as usize;
            let relation_size = u32_at(&data, sh + 28)? as usize;
            let records_end = u32_at(&data, sh + 20)? as usize;
            let offset_map_count = u32_at(&data, sh + 32)? as usize;
            let copy_count = u32_at(&data, sh + 36)? as usize;
            if sparse {
                // The sparse data, then the IDs, copies, offset map,
                // relationship map and the offset map's IDs.
                let ids_at = records_end;
                let copy_at = ids_at + id_list_size;
                let map_at = copy_at + copy_count * 8;
                let ids_of_map = map_at + offset_map_count * 6 + relation_size;
                for r in 0..offset_map_count {
                    let at = u32_at(&data, map_at + r * 6)? as usize;
                    let size = usize::from(u16_at(&data, map_at + r * 6 + 4)?);
                    if size == 0 {
                        continue;
                    }
                    let rec = data.get(at..at + size)?;
                    if tact_key != 0 && rec.iter().all(|&b| b == 0) {
                        continue;
                    }
                    let id = if id_list_size > 0 {
                        u32_at(&data, ids_at + r * 4)?
                    } else {
                        u32_at(&data, ids_of_map + r * 4)?
                    };
                    records.push(Record {
                        at,
                        virtual_at: 0,
                        id,
                        relation: 0,
                        size,
                        hot: false,
                    });
                }
                for c in 0..copy_count {
                    copies.push((
                        u32_at(&data, copy_at + c * 8)?,
                        u32_at(&data, copy_at + c * 8 + 4)?,
                    ));
                }
                continue;
            }
            let mut section_records: Vec<Option<usize>> = Vec::with_capacity(count);
            let strings_at = file_offset + count * record_size;
            strings.push((strings_at, string_size));
            let ids_at = strings_at + string_size;
            let copy_at = ids_at + id_list_size;
            for r in 0..count {
                let at = file_offset + r * record_size;
                let rec = data.get(at..at + record_size)?;
                // An encrypted section the client has no key for: zeros.
                if tact_key != 0 && rec.iter().all(|&b| b == 0) {
                    virtual_at += record_size;
                    section_records.push(None);
                    continue;
                }
                section_records.push(Some(records.len()));
                let id = if id_list_size > 0 {
                    u32_at(&data, ids_at + r * 4)?
                } else {
                    id_from_field.push(records.len());
                    0
                };
                records.push(Record {
                    at,
                    virtual_at,
                    id,
                    relation: 0,
                    size: record_size,
                    hot: false,
                });
                virtual_at += record_size;
            }
            for c in 0..copy_count {
                copies.push((
                    u32_at(&data, copy_at + c * 8)?,
                    u32_at(&data, copy_at + c * 8 + 4)?,
                ));
            }
            // The relationship map: (parent ID, record index) pairs.
            let rel_at = copy_at + copy_count * 8 + offset_map_count * 6;
            if relation_size >= 12 {
                let n = u32_at(&data, rel_at)? as usize;
                for k in 0..n {
                    let e = rel_at + 12 + k * 8;
                    let (Some(parent), Some(index)) = (u32_at(&data, e), u32_at(&data, e + 4))
                    else {
                        break;
                    };
                    if let Some(Some(r)) = section_records.get(index as usize) {
                        records[*r].relation = parent;
                    }
                }
            }
        }
        let mut db = Db2 {
            data,
            fields,
            records,
            pallet: (pallet_at, pallet_size),
            common: (common_at, common_size),
            strings,
            records_bytes: virtual_at,
            layout_hash,
            table_hash,
            hot: Vec::new(),
            hot_sizes: Vec::new(),
            natural: Vec::new(),
            by_id: std::sync::OnceLock::new(),
            sparse,
            string_fields: Vec::new(),
        };
        for i in id_from_field {
            db.records[i].id = db.int(i, id_index).unwrap_or(0);
        }
        // Copies: another record's fields under a new ID.
        if !copies.is_empty() {
            let by_id: std::collections::HashMap<u32, Record> =
                db.records.iter().map(|r| (r.id, *r)).collect();
            for (new_id, old_id) in copies {
                if let Some(r) = by_id.get(&old_id) {
                    db.records.push(Record { id: new_id, ..*r });
                }
            }
        }
        Some(db)
    }

    /// Which fields of a sparse table are strings (they are stored inline
    /// there, so the fields after them move with them).
    pub fn set_string_fields(&mut self, strings: Vec<bool>) {
        self.string_fields = strings;
    }

    pub fn is_sparse(&self) -> bool {
        self.sparse
    }

    /// A sparse record's field `f`: where it starts in the file.
    fn sparse_at(&self, r: &Record, f: usize) -> Option<usize> {
        let mut at = r.at;
        let end = r.at + r.size;
        for j in 0..f {
            if self.string_fields.get(j).copied().unwrap_or(false) {
                let s = self.data.get(at..end)?;
                at += s.iter().position(|&b| b == 0)? + 1;
            } else {
                let field = self.fields.get(j)?;
                let size = field.byte_size.clamp(1, 4);
                let count = (field.size_bits as usize / (size * 8)).max(1);
                at += size * count;
            }
        }
        (at <= end).then_some(at)
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Each field's storage, bits, byte size and element count (for tools).
    pub fn field_info(&self) -> Vec<(u32, u32, usize, usize)> {
        self.fields
            .iter()
            .map(|f| (f.storage, f.size_bits, f.byte_size, Self::count(f)))
            .collect()
    }

    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    pub fn id(&self, i: usize) -> u32 {
        self.records[i].id
    }

    /// Record `i`'s parent ID from the relationship map (0: none).
    pub fn relation(&self, i: usize) -> u32 {
        self.records[i].relation
    }

    /// Record `i`'s field `f` as an integer.
    pub fn int(&self, i: usize, f: usize) -> Option<u32> {
        let r = &self.records[i];
        if r.hot {
            return self.hot_ints(r, f)?.first().copied();
        }
        self.field_in(&self.data, r, f).map(|v| self.cut(f, v))
    }

    /// The layout's field sizes (`None` where unknown), for cutting stored
    /// values to them.
    pub fn set_field_sizes(&mut self, sizes: Vec<Option<usize>>) {
        self.natural = sizes;
    }

    /// `v` cut to field `f`'s size.
    fn cut(&self, f: usize, v: u32) -> u32 {
        match self.natural.get(f).copied().flatten() {
            Some(1) => v & 0xFF,
            Some(2) => v & 0xFFFF,
            _ => v,
        }
    }

    /// How many elements field `f` has (an array's length).
    fn count(field: &Field) -> usize {
        let size = field.byte_size.clamp(1, 4);
        match field.storage {
            0 => (field.size_bits as usize / (size * 8)).max(1),
            4 => field.val3.max(1) as usize,
            _ => 1,
        }
    }

    /// A hotfix record's field `f`: where it starts in `hot`.
    fn hot_at(&self, r: &Record, f: usize) -> Option<usize> {
        let mut at = r.at;
        let end = r.at + r.size;
        for j in 0..f {
            if self.string_fields.get(j).copied().unwrap_or(false) {
                let s = self.hot.get(at..end)?;
                at += s.iter().position(|&b| b == 0)? + 1;
            } else {
                let field = self.fields.get(j)?;
                at += self.hot_sizes.get(j)? * Self::count(field);
            }
        }
        (at <= end).then_some(at)
    }

    fn hot_ints(&self, r: &Record, f: usize) -> Option<Vec<u32>> {
        let field = self.fields.get(f)?;
        let size = (*self.hot_sizes.get(f)?).clamp(1, 4);
        let start = self.hot_at(r, f)?;
        (0..Self::count(field))
            .map(|k| {
                let at = start + k * size;
                let mut buf = [0u8; 4];
                buf[..size].copy_from_slice(self.hot.get(at..at + size)?);
                Some(u32::from_le_bytes(buf))
            })
            .collect()
    }

    /// Takes the client's hotfixes of this table (`Casc::hotfixes`): a
    /// valid record (status 1) replaces the record of its ID (keeping its
    /// parent in a relationship map) or is added, a deleted one (2) is
    /// removed. `sizes`: each field's size in bytes as hotfixes store it;
    /// without one for a field that is not stored whole, the hotfixes are
    /// not taken. Needs [`Db2::set_string_fields`] first.
    pub fn apply_hotfixes(&mut self, rows: &[(u32, u8, Vec<u8>)], sizes: &[Option<usize>]) {
        if rows.is_empty() {
            return;
        }
        let mut hot_sizes = Vec::with_capacity(self.fields.len());
        for (j, f) in self.fields.iter().enumerate() {
            let string = self.string_fields.get(j).copied().unwrap_or(false);
            match sizes.get(j).copied().flatten() {
                Some(n) => hot_sizes.push(n),
                None if string || (f.storage == 0 && f.byte_size > 0) => {
                    hot_sizes.push(f.byte_size.clamp(1, 4))
                }
                None => return,
            }
        }
        self.hot_sizes = hot_sizes;
        let mut changed = false;
        for (id, status, data) in rows {
            let old = self.records.iter().position(|r| r.id == *id);
            match status {
                1 => {
                    let at = self.hot.len();
                    self.hot.extend_from_slice(data);
                    let r = Record {
                        at,
                        virtual_at: 0,
                        id: *id,
                        relation: old.map_or(0, |i| self.records[i].relation),
                        size: data.len(),
                        hot: true,
                    };
                    // (a record that does not fit the layout is left out;
                    // four bytes more are its parent's ID)
                    let last = self.fields.len().saturating_sub(1);
                    let Some(end) = self.hot_at(&r, last).and_then(|at| {
                        if self.string_fields.get(last).copied().unwrap_or(false) {
                            let s = self.hot.get(at..r.at + r.size)?;
                            Some(at + s.iter().position(|&b| b == 0)? + 1)
                        } else {
                            Some(at + self.hot_sizes[last] * Self::count(&self.fields[last]))
                        }
                    }) else {
                        continue;
                    };
                    let mut r = r;
                    if end + 4 == r.at + r.size {
                        r.relation = u32_at(&self.hot, end).unwrap_or(r.relation);
                    } else if end != r.at + r.size {
                        continue;
                    }
                    if self.fields.is_empty() {
                        continue;
                    }
                    match old {
                        Some(i) => self.records[i] = r,
                        None => self.records.push(r),
                    }
                    changed = true;
                }
                2 => {
                    if let Some(i) = old {
                        self.records.remove(i);
                        changed = true;
                    }
                }
                _ => {}
            }
        }
        if changed {
            self.by_id = std::sync::OnceLock::new();
        }
    }

    fn field_in(&self, data: &[u8], r: &Record, f: usize) -> Option<u32> {
        let field = self.fields.get(f)?;
        if self.sparse {
            let at = self.sparse_at(r, f)?;
            let mut buf = [0u8; 4];
            let n = field.byte_size.clamp(1, 4);
            buf[..n].copy_from_slice(data.get(at..at + n)?);
            return Some(u32::from_le_bytes(buf));
        }
        let bits = |off: u32, size: u32| -> Option<u32> {
            if size == 0 {
                return Some(0);
            }
            let start = r.at + (off / 8) as usize;
            let mut buf = [0u8; 8];
            let n = (((off % 8) + size).div_ceil(8)) as usize;
            buf[..n].copy_from_slice(data.get(start..start + n)?);
            let v = u64::from_le_bytes(buf) >> (off % 8);
            Some((v & ((1u64 << size) - 1)) as u32)
        };
        match field.storage {
            0 => {
                let at = r.at + field.byte_offset;
                let mut buf = [0u8; 4];
                let n = field.byte_size.clamp(1, 4);
                buf[..n].copy_from_slice(data.get(at..at + n)?);
                Some(u32::from_le_bytes(buf))
            }
            1 | 5 => bits(field.offset_bits, field.size_bits),
            2 => {
                let (at, _) = self.common;
                let start = at + field.extra;
                for k in 0..field.extra_len / 8 {
                    if u32_at(data, start + k * 8)? == r.id {
                        return u32_at(data, start + k * 8 + 4);
                    }
                }
                Some(field.val1)
            }
            3 | 4 => {
                let index = bits(field.offset_bits, field.size_bits)? as usize;
                let len = if field.storage == 4 {
                    field.val3.max(1) as usize
                } else {
                    1
                };
                let (at, _) = self.pallet;
                u32_at(data, at + field.extra + index * len * 4)
            }
            _ => None,
        }
    }

    /// Record `i`'s array field `f`: all its elements (one for a field
    /// that is not an array).
    pub fn ints(&self, i: usize, f: usize) -> Vec<u32> {
        let Some(field) = self.fields.get(f) else {
            return Vec::new();
        };
        let r = &self.records[i];
        if r.hot {
            return self.hot_ints(r, f).unwrap_or_default();
        }
        let data = &self.data;
        if self.sparse {
            let size = field.byte_size.clamp(1, 4);
            let count = (field.size_bits as usize / (size * 8)).max(1);
            let Some(start) = self.sparse_at(r, f) else {
                return Vec::new();
            };
            return (0..count)
                .filter_map(|k| {
                    let at = start + k * size;
                    let mut buf = [0u8; 4];
                    buf[..size].copy_from_slice(data.get(at..at + size)?);
                    Some(u32::from_le_bytes(buf))
                })
                .collect();
        }
        match field.storage {
            0 => {
                let size = field.byte_size.clamp(1, 4);
                let count = (field.size_bits as usize / (size * 8)).max(1);
                (0..count)
                    .filter_map(|k| {
                        let at = r.at + field.byte_offset + k * size;
                        let mut buf = [0u8; 4];
                        buf[..size].copy_from_slice(data.get(at..at + size)?);
                        Some(u32::from_le_bytes(buf))
                    })
                    .collect()
            }
            4 => {
                let len = field.val3.max(1) as usize;
                let Some(index) = self.bits_of(r, field) else {
                    return Vec::new();
                };
                let (at, _) = self.pallet;
                (0..len)
                    .filter_map(|k| u32_at(data, at + field.extra + (index as usize * len + k) * 4))
                    .map(|v| self.cut(f, v))
                    .collect()
            }
            _ => self.int(i, f).into_iter().collect(),
        }
    }

    /// A bit-packed field's raw value.
    fn bits_of(&self, r: &Record, field: &Field) -> Option<u32> {
        if field.size_bits == 0 {
            return Some(0);
        }
        let off = field.offset_bits;
        let start = r.at + (off / 8) as usize;
        let mut buf = [0u8; 8];
        let n = (((off % 8) + field.size_bits).div_ceil(8)) as usize;
        buf[..n].copy_from_slice(self.data.get(start..start + n)?);
        let v = u64::from_le_bytes(buf) >> (off % 8);
        Some((v & ((1u64 << field.size_bits) - 1)) as u32)
    }

    /// Record `i`'s field `f` as a float.
    pub fn float(&self, i: usize, f: usize) -> Option<f32> {
        self.int(i, f).map(f32::from_bits)
    }

    /// Record `i`'s field `f` as a signed integer (a bit-packed signed
    /// field's sign extended).
    pub fn signed(&self, i: usize, f: usize) -> Option<i32> {
        let field = self.fields.get(f)?;
        let v = self.int(i, f)?;
        match self.natural.get(f).copied().flatten() {
            Some(1) => return Some(v as u8 as i8 as i32),
            Some(2) => return Some(v as u16 as i16 as i32),
            _ => {}
        }
        if field.storage == 5 && field.size_bits > 0 && field.size_bits < 32 {
            let shift = 32 - field.size_bits;
            return Some(((v << shift) as i32) >> shift);
        }
        Some(match field.byte_size {
            1 if field.storage == 0 => v as u8 as i8 as i32,
            2 if field.storage == 0 => v as u16 as i16 as i32,
            _ => v as i32,
        })
    }

    /// The index of the record with ID `id`.
    pub fn find(&self, id: u32) -> Option<usize> {
        self.by_id
            .get_or_init(|| {
                let mut m = std::collections::HashMap::with_capacity(self.records.len());
                for (i, r) in self.records.iter().enumerate() {
                    m.entry(r.id).or_insert(i);
                }
                m
            })
            .get(&id)
            .copied()
    }

    /// Record `i`'s string field `f`.
    pub fn string(&self, i: usize, f: usize) -> Option<String> {
        let r = &self.records[i];
        if r.hot {
            let at = self.hot_at(r, f)?;
            let s = self.hot.get(at..r.at + r.size)?;
            let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
            return Some(String::from_utf8_lossy(&s[..end]).into_owned());
        }
        if self.sparse {
            let at = self.sparse_at(r, f)?;
            let s = self.data.get(at..r.at + r.size)?;
            let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
            return Some(String::from_utf8_lossy(&s[..end]).into_owned());
        }
        let field = self.fields.get(f)?;
        let off = self.int(i, f)? as usize;
        let virtual_pos = r.virtual_at + field.byte_offset + off;
        let mut pos = virtual_pos.checked_sub(self.records_bytes)?;
        for &(at, size) in &self.strings {
            if pos < size {
                let s = self.data.get(at + pos..at + size)?;
                let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
                return Some(String::from_utf8_lossy(&s[..end]).into_owned());
            }
            pos -= size;
        }
        None
    }
}
