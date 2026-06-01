use anyhow::{Result, bail};

pub fn put_u32(mut value: u32, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

pub fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32> {
    let mut shift = 0;
    let mut value = 0u32;

    while *cursor < bytes.len() {
        let byte = bytes[*cursor];
        *cursor += 1;
        value |= ((byte & 0x7f) as u32) << shift;

        if byte & 0x80 == 0 {
            return Ok(value);
        }

        shift += 7;
        if shift >= 35 {
            bail!("u32 varint overflow");
        }
    }

    bail!("truncated u32 varint");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_round_trips() {
        let values = [0, 1, 2, 127, 128, 255, 16_384, u32::MAX];
        let mut bytes = Vec::new();
        for value in values {
            put_u32(value, &mut bytes);
        }

        let mut cursor = 0;
        for value in values {
            assert_eq!(read_u32(&bytes, &mut cursor).unwrap(), value);
        }
        assert_eq!(cursor, bytes.len());
    }
}
