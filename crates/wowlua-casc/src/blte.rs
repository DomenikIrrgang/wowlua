//! BLTE: the storage's encoded files, in chunks, each raw (`N`), zlib
//! (`Z`), nested (`F`) or encrypted (`E`, Salsa20 with a key of the
//! client's own, read when the installation has it).

use std::collections::HashMap;
use std::io::Read;

/// The client's encryption keys, by name.
pub type Keys = HashMap<u64, [u8; 16]>;

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn chunk(data: &[u8], index: usize, keys: &Keys, out: &mut Vec<u8>) -> Result<(), String> {
    let Some((&mode, body)) = data.split_first() else {
        return Ok(());
    };
    match mode {
        b'N' => out.extend_from_slice(body),
        b'Z' => {
            flate2::read::ZlibDecoder::new(body)
                .read_to_end(out)
                .map_err(|e| format!("a zlib chunk: {e}"))?;
        }
        b'F' => out.extend_from_slice(&decode(body, keys)?),
        b'E' => {
            let plain = decrypt(body, index, keys)?;
            chunk(&plain, index, keys, out)?;
        }
        m => return Err(format!("a chunk of mode {:?}", m as char)),
    }
    Ok(())
}

/// An encrypted chunk's body: the key's name, the IV, the cipher, the
/// data.
fn decrypt(body: &[u8], index: usize, keys: &Keys) -> Result<Vec<u8>, String> {
    let short = || "a short encrypted chunk".to_string();
    let name_len = *body.first().ok_or_else(short)? as usize;
    if name_len != 8 {
        return Err(format!(
            "an encrypted chunk with a {name_len}-byte key name"
        ));
    }
    let name = u64::from_le_bytes(body.get(1..9).ok_or_else(short)?.try_into().unwrap());
    let iv_len = *body.get(9).ok_or_else(short)? as usize;
    let iv = body.get(10..10 + iv_len).ok_or_else(short)?;
    let cipher = *body.get(10 + iv_len).ok_or_else(short)?;
    let data = body.get(11 + iv_len..).ok_or_else(short)?;
    let Some(key) = keys.get(&name) else {
        return Err(format!("an encrypted chunk (key {name:016X}, not known)"));
    };
    if cipher != b'S' {
        return Err(format!("an encrypted chunk of cipher {:?}", cipher as char));
    }
    let mut nonce = [0u8; 8];
    nonce[..iv.len().min(8)].copy_from_slice(&iv[..iv.len().min(8)]);
    for (i, n) in nonce.iter_mut().take(4).enumerate() {
        *n ^= (index >> (i * 8)) as u8;
    }
    let mut plain = data.to_vec();
    crate::salsa20::apply(key, &nonce, &mut plain);
    Ok(plain)
}

/// Decodes a BLTE file.
pub fn decode(data: &[u8], keys: &Keys) -> Result<Vec<u8>, String> {
    if data.len() < 8 || &data[..4] != b"BLTE" {
        return Err("not BLTE".into());
    }
    let header = be32(&data[4..8]) as usize;
    let mut out = Vec::new();
    if header == 0 {
        chunk(&data[8..], 0, keys, &mut out)?;
        return Ok(out);
    }
    if data.len() < 12 || header > data.len() {
        return Err("a short BLTE header".into());
    }
    let count =
        (u32::from(data[9]) << 16 | u32::from(data[10]) << 8 | u32::from(data[11])) as usize;
    let mut pos = header;
    let mut table = 12;
    for index in 0..count {
        if table + 24 > header {
            return Err("a short chunk table".into());
        }
        let compressed = be32(&data[table..]) as usize;
        let decompressed = be32(&data[table + 4..]) as usize;
        table += 24;
        let end = pos + compressed;
        if end > data.len() {
            return Err("a chunk past the file's end".into());
        }
        out.reserve(decompressed);
        // A chunk encrypted with a key the client lacks reads as zeros, as
        // in the client: a table's encrypted section is such a chunk.
        if data[pos] == b'E' && decrypt(&data[pos + 1..end], index, keys).is_err() {
            out.resize(out.len() + decompressed, 0);
        } else {
            chunk(&data[pos..end], index, keys, &mut out)?;
        }
        pos = end;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn decodes_raw_and_zlib_chunks() {
        let single = [b"BLTE".as_slice(), &[0, 0, 0, 0], b"Nhello"].concat();
        assert_eq!(decode(&single, &Keys::new()).unwrap(), b"hello");
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(b" world").unwrap();
        let z = [b"Z".as_slice(), &z.finish().unwrap()].concat();
        let raw = b"Nhello".to_vec();
        let header = 12 + 2 * 24;
        let mut f = b"BLTE".to_vec();
        f.extend_from_slice(&(header as u32).to_be_bytes());
        f.extend_from_slice(&[0x0f, 0, 0, 2]);
        for (c, d) in [(&raw, 5u32), (&z, 6u32)] {
            f.extend_from_slice(&(c.len() as u32).to_be_bytes());
            f.extend_from_slice(&d.to_be_bytes());
            f.extend_from_slice(&[0; 16]);
        }
        f.extend_from_slice(&raw);
        f.extend_from_slice(&z);
        assert_eq!(decode(&f, &Keys::new()).unwrap(), b"hello world");
    }

    /// An encrypted chunk: read with its key; without it, zeros of the
    /// chunk's size.
    #[test]
    fn decrypts_chunks_with_known_keys() {
        let key = [7u8; 16];
        let name = 0x1122_3344_5566_7788u64;
        let iv = [1u8, 2, 3, 4];
        let mut inner = b"Nsecret".to_vec();
        // Chunk 0: the nonce is the IV as it is.
        crate::salsa20::apply(&key, &[1, 2, 3, 4, 0, 0, 0, 0], &mut inner);
        let mut chunk = vec![b'E', 8];
        chunk.extend_from_slice(&name.to_le_bytes());
        chunk.push(4);
        chunk.extend_from_slice(&iv);
        chunk.push(b'S');
        chunk.extend_from_slice(&inner);
        let header = 12 + 24;
        let mut f = b"BLTE".to_vec();
        f.extend_from_slice(&(header as u32).to_be_bytes());
        f.extend_from_slice(&[0x0f, 0, 0, 1]);
        f.extend_from_slice(&(chunk.len() as u32).to_be_bytes());
        f.extend_from_slice(&6u32.to_be_bytes());
        f.extend_from_slice(&[0; 16]);
        f.extend_from_slice(&chunk);
        let mut keys = Keys::new();
        assert_eq!(decode(&f, &keys).unwrap(), vec![0; 6]);
        keys.insert(name, key);
        assert_eq!(decode(&f, &keys).unwrap(), b"secret");
    }
}
