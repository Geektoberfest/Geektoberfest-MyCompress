//! CRC32 checksums (IEEE 802.3 polynomial, the same variant used by zip and
//! PNG). Implemented in-crate so that the core library stays dependency-free.

const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut value = i as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 != 0 {
                0xEDB8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[i] = value;
        i += 1;
    }
    table
}

static TABLE: [u32; 256] = build_table();

/// Computes the CRC32 (IEEE) checksum of `data`.
pub fn crc32(data: &[u8]) -> u32 {
    let mut hasher = Crc32::new();
    hasher.update(data);
    hasher.finalize()
}

/// Incremental CRC32 hasher for streaming inputs.
///
/// Feeding `data` through one `Crc32` in chunks yields the same value as
/// [`crc32`] over the concatenation.
#[derive(Clone, Copy, Debug)]
pub struct Crc32 {
    state: u32,
}

impl Crc32 {
    /// Starts a new checksum computation.
    #[must_use]
    pub fn new() -> Self {
        Self { state: 0xFFFF_FFFF }
    }

    /// Folds a chunk of input into the running checksum.
    pub fn update(&mut self, data: &[u8]) {
        let mut crc = self.state;
        for &byte in data {
            crc = TABLE[((crc ^ u32::from(byte)) & 0xFF) as usize] ^ (crc >> 8);
        }
        self.state = crc;
    }

    /// Returns the finished checksum.
    #[must_use]
    pub fn finalize(self) -> u32 {
        self.state ^ 0xFFFF_FFFF
    }
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
        assert_eq!(crc32(b"abc"), 0x3524_41C2);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414F_A339
        );
    }

    #[test]
    fn is_incremental_in_input_length() {
        // Changing one byte must change the checksum.
        assert_ne!(crc32(b"hello"), crc32(b"hellp"));
        // Prefix extension must change the checksum.
        assert_ne!(crc32(b"hello"), crc32(b"hello!"));
    }

    #[test]
    fn streaming_matches_one_shot() {
        let data = b"The quick brown fox jumps over the lazy dog";
        let mut chunked = Crc32::new();
        for chunk in data.chunks(7) {
            chunked.update(chunk);
        }
        assert_eq!(chunked.finalize(), crc32(data));

        let mut empty = Crc32::new();
        empty.update(b"");
        assert_eq!(empty.finalize(), crc32(b""));
        assert_eq!(Crc32::default().finalize(), crc32(b""));
    }
}
