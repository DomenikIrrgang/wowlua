//! BLTE: the storage's encoded files, in chunks, each raw (`N`), zlib
//! (`Z`), nested (`F`) or encrypted (`E`, not read: the keys are not ours).

use std::io::Read;

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn chunk(data: &[u8], out: &mut Vec<u8>) -> Result<(), String> {
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
        b'F' => out.extend_from_slice(&decode(body)?),
        b'E' => return Err("an encrypted chunk".into()),
        m => return Err(format!("a chunk of mode {:?}", m as char)),
    }
    Ok(())
}

/// Decodes a BLTE file.
pub fn decode(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 8 || &data[..4] != b"BLTE" {
        return Err("not BLTE".into());
    }
    let header = be32(&data[4..8]) as usize;
    let mut out = Vec::new();
    if header == 0 {
        chunk(&data[8..], &mut out)?;
        return Ok(out);
    }
    if data.len() < 12 || header > data.len() {
        return Err("a short BLTE header".into());
    }
    let count =
        (u32::from(data[9]) << 16 | u32::from(data[10]) << 8 | u32::from(data[11])) as usize;
    let mut pos = header;
    let mut table = 12;
    for _ in 0..count {
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
        chunk(&data[pos..end], &mut out)?;
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
        assert_eq!(decode(&single).unwrap(), b"hello");
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
        assert_eq!(decode(&f).unwrap(), b"hello world");
    }
}
