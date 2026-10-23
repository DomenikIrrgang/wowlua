//! The client's tables (DB2, the WDC4 and WDC5 formats): records of
//! fields, each stored inline, bit-packed, in a palette or as a default
//! with exceptions. Read by field index; strings by their offsets into the
//! sections' string tables.

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
    by_id: std::sync::OnceLock<std::collections::HashMap<u32, usize>>,
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
        let layout_hash = u(5)?;
        let flags = u16_at(&data, h + 36)?;
        let id_index = u16_at(&data, h + 38)? as usize;
        let total_fields = u32_at(&data, h + 40)? as usize;
        let fsi_size = u32_at(&data, h + 52)? as usize;
        let common_size = u32_at(&data, h + 56)? as usize;
        let pallet_size = u32_at(&data, h + 60)? as usize;
        let section_count = u32_at(&data, h + 64)? as usize;
        let _ = (record_count, field_count);
        if flags & 1 != 0 {
            // Variable-size records (an offset map): not read here.
            return None;
        }
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
            let offset_map_count = u32_at(&data, sh + 32)? as usize;
            let copy_count = u32_at(&data, sh + 36)? as usize;
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
            by_id: std::sync::OnceLock::new(),
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

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
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
        self.field_in(&self.data, &self.records[i], f)
    }

    fn field_in(&self, data: &[u8], r: &Record, f: usize) -> Option<u32> {
        let field = self.fields.get(f)?;
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
        let data = &self.data;
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
