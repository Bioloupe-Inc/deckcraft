//! Fonts embedded in Office files. PowerPoint stores them as Embedded OpenType (`.fntdata`; W3C
//! submission "Embedded OpenType (EOT) File Format", 2008): a header, then the font data, plain or
//! XOR-obscured. Some writers store the plain font file instead.

/// `Flags` bit: the font data is MicroType Express compressed.
const TTEMBED_TTCOMPRESSED: u32 = 0x4;
/// `Flags` bit: every byte of the font data is XORed with 0x50.
const TTEMBED_XORENCRYPTDATA: u32 = 0x1000_0000;
const MAGIC: u16 = 0x504C;

fn is_sfnt(b: &[u8]) -> bool {
    matches!(b.get(..4), Some([0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf"))
}

/// The font file (TTF, OTF or TTC) inside an embedded font: the bytes themselves when they are
/// one, the font data of an uncompressed EOT, or `None` (MicroType Express compression, which
/// isn't supported, or not a font).
pub fn embedded_font_file(bytes: &[u8]) -> Option<Vec<u8>> {
    if is_sfnt(bytes) {
        return Some(bytes.to_vec());
    }
    let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let (size, data_size, flags) = (u32_at(0)? as usize, u32_at(4)? as usize, u32_at(12)?);
    let magic = bytes.get(34..36).map(|b| u16::from_le_bytes([b[0], b[1]]))?;
    if magic != MAGIC || flags & TTEMBED_TTCOMPRESSED != 0 || size > bytes.len() || data_size > size {
        return None;
    }
    // The font data is the record's last field in every version.
    let mut data = bytes[size - data_size..size].to_vec();
    if flags & TTEMBED_XORENCRYPTDATA != 0 {
        data.iter_mut().for_each(|b| *b ^= 0x50);
    }
    is_sfnt(&data).then_some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eot(font: &[u8], flags: u32) -> Vec<u8> {
        let mut v = vec![0u8; 82];
        let size = (v.len() + font.len()) as u32;
        v[0..4].copy_from_slice(&size.to_le_bytes());
        v[4..8].copy_from_slice(&(font.len() as u32).to_le_bytes());
        v[8..12].copy_from_slice(&0x0002_0001u32.to_le_bytes());
        v[12..16].copy_from_slice(&flags.to_le_bytes());
        v[34..36].copy_from_slice(&MAGIC.to_le_bytes());
        v.extend(font.iter().map(|b| if flags & TTEMBED_XORENCRYPTDATA != 0 { b ^ 0x50 } else { *b }));
        v
    }

    #[test]
    fn reads_plain_obscured_and_bare_fonts() {
        let font = [0u8, 1, 0, 0, 7, 7, 7];
        assert_eq!(embedded_font_file(&eot(&font, 0)).as_deref(), Some(&font[..]));
        assert_eq!(embedded_font_file(&eot(&font, TTEMBED_XORENCRYPTDATA)).as_deref(), Some(&font[..]));
        assert_eq!(embedded_font_file(&font).as_deref(), Some(&font[..]));
    }

    #[test]
    fn refuses_compressed_and_foreign_data() {
        assert_eq!(embedded_font_file(&eot(&[0, 1, 0, 0], TTEMBED_TTCOMPRESSED)), None);
        assert_eq!(embedded_font_file(b"not a font at all, not even close to one"), None);
    }
}
