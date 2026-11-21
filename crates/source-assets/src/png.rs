//! Writing screenshots as PNG: 8-bit RGB with every row unfiltered and stored in uncompressed
//! deflate blocks. A reader takes the pixels row by row without undoing filters or inflating
//! Huffman codes, at the cost of files the size of the raw pixels; any PNG reader opens them.

/// The largest stored deflate block (its length is a 16-bit field).
const STORED_BLOCK: usize = 65_535;

/// The PNG file of `width` x `height` 8-bit RGB pixels, `rgb` holding them row by row.
pub fn encode_rgb(width: u32, height: u32, rgb: &[u8]) -> Result<Vec<u8>, String> {
    let stride = width as usize * 3;
    if rgb.len() != stride * height as usize {
        return Err(format!(
            "{} RGB bytes for {width} x {height} pixels",
            rgb.len()
        ));
    }
    // Each scanline starts with its filter type: 0, none.
    let mut scanlines = Vec::with_capacity((stride + 1) * height as usize);
    for row in rgb.chunks_exact(stride.max(1)).take(height as usize) {
        scanlines.push(0);
        scanlines.extend_from_slice(row);
    }
    let mut stream = Vec::with_capacity(scanlines.len() + scanlines.len() / STORED_BLOCK * 5 + 11);
    // zlib header: deflate with a 32 KiB window, no preset dictionary (0x78 0x01 passes the
    // FCHECK test).
    stream.extend_from_slice(&[0x78, 0x01]);
    let blocks: Vec<&[u8]> = if scanlines.is_empty() {
        vec![&[]]
    } else {
        scanlines.chunks(STORED_BLOCK).collect()
    };
    for (index, block) in blocks.iter().enumerate() {
        // BFINAL on the last block, BTYPE 00 (stored), then LEN and its one's complement.
        stream.push(u8::from(index + 1 == blocks.len()));
        let length = block.len() as u16;
        stream.extend_from_slice(&length.to_le_bytes());
        stream.extend_from_slice(&(!length).to_le_bytes());
        stream.extend_from_slice(block);
    }
    stream.extend_from_slice(&adler32(&scanlines).to_be_bytes());
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    // Bit depth 8, colour type 2 (truecolour), deflate, filter method 0, no interlace.
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut png, b"IHDR", &header);
    chunk(&mut png, b"IDAT", &stream);
    chunk(&mut png, b"IEND", &[]);
    Ok(png)
}

/// Appends a PNG chunk: length, type, data and the CRC-32 of type and data.
fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    png.extend_from_slice(&crc.finalize().to_be_bytes());
}

/// The zlib checksum (RFC 1950), reduced every 5552 bytes so the sums never overflow.
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for run in data.chunks(5552) {
        for &byte in run {
            a += u32::from(byte);
            b += a;
        }
        a %= 65_521;
        b %= 65_521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The chunks of a PNG file: type and data, each CRC checked.
    fn chunks(png: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let mut found = Vec::new();
        let mut offset = 8;
        while offset < png.len() {
            let length = u32::from_be_bytes(png[offset..offset + 4].try_into().unwrap()) as usize;
            let kind: [u8; 4] = png[offset + 4..offset + 8].try_into().unwrap();
            let data = png[offset + 8..offset + 8 + length].to_vec();
            let stored = u32::from_be_bytes(
                png[offset + 8 + length..offset + 12 + length]
                    .try_into()
                    .unwrap(),
            );
            let mut crc = crc32fast::Hasher::new();
            crc.update(&kind);
            crc.update(&data);
            assert_eq!(crc.finalize(), stored, "CRC of {kind:?}");
            found.push((kind, data));
            offset += length + 12;
        }
        found
    }

    /// The bytes inside a zlib stream of stored blocks, checking the header, block lengths and
    /// the Adler-32 trailer.
    fn inflate_stored(stream: &[u8]) -> Vec<u8> {
        assert_eq!(u16::from_be_bytes([stream[0], stream[1]]) % 31, 0, "FCHECK");
        let mut out = Vec::new();
        let mut offset = 2;
        loop {
            let last = stream[offset] & 1 == 1;
            assert_eq!(stream[offset] >> 1, 0, "stored blocks only");
            let length = u16::from_le_bytes([stream[offset + 1], stream[offset + 2]]);
            let complement = u16::from_le_bytes([stream[offset + 3], stream[offset + 4]]);
            assert_eq!(length, !complement);
            out.extend_from_slice(&stream[offset + 5..offset + 5 + length as usize]);
            offset += 5 + length as usize;
            if last {
                break;
            }
        }
        assert_eq!(
            u32::from_be_bytes(stream[offset..offset + 4].try_into().unwrap()),
            adler32(&out)
        );
        out
    }

    #[test]
    fn adler32_matches_the_rfc_example() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(adler32(&[]), 1);
    }

    #[test]
    fn rows_are_stored_unfiltered_in_valid_chunks() {
        let rgb: Vec<u8> = (0..2 * 3 * 3).map(|i| i as u8).collect();
        let png = encode_rgb(3, 2, &rgb).unwrap();
        let chunks = chunks(&png);
        let kinds: Vec<&[u8; 4]> = chunks.iter().map(|(kind, _)| kind).collect();
        assert_eq!(kinds, [b"IHDR", b"IDAT", b"IEND"]);
        assert_eq!(chunks[0].1, [0, 0, 0, 3, 0, 0, 0, 2, 8, 2, 0, 0, 0]);
        let scanlines = inflate_stored(&chunks[1].1);
        assert_eq!(scanlines[0], 0);
        assert_eq!(&scanlines[1..10], &rgb[..9]);
        assert_eq!(scanlines[10], 0);
        assert_eq!(&scanlines[11..], &rgb[9..]);
    }

    #[test]
    fn large_images_span_several_stored_blocks() {
        let (width, height) = (300, 200);
        let rgb: Vec<u8> = (0..width * height * 3).map(|i| (i * 7) as u8).collect();
        let png = encode_rgb(width as u32, height as u32, &rgb).unwrap();
        let scanlines = inflate_stored(&chunks(&png)[1].1);
        assert!(scanlines.len() > STORED_BLOCK);
        let rows: Vec<u8> = scanlines
            .chunks(width * 3 + 1)
            .flat_map(|line| {
                assert_eq!(line[0], 0);
                line[1..].to_vec()
            })
            .collect();
        assert_eq!(rows, rgb);
    }

    #[test]
    fn a_wrong_pixel_count_is_an_error() {
        assert!(encode_rgb(2, 2, &[0; 11]).is_err());
    }
}
