# `.mcz` Archive Format

Version 1 archives use little-endian integers and contain a 16-byte header
followed by zero or more entries. The reader validates the complete structure
before extraction and verifies every file's decompressed size and CRC32.

## Header

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 4 | Magic bytes `MCZ1` |
| 4 | 2 | Format version (`u16`), currently `1` |
| 6 | 2 | Compression algorithm ID (`u16`) |
| 8 | 4 | Entry count (`u32`) |
| 12 | 4 | Reserved, must be zero |

Built-in algorithm IDs:
- `1`: Run-Length Encoding (RLE)
- `2`: Canonical Huffman coding

IDs `1..1023` are reserved for built-in algorithms; IDs `>= 1024` are available
for custom third-party algorithms.

## Entry

Each entry has a 36-byte fixed record, followed by its UTF-8 path and
compressed data:

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 2 | Path length in bytes (`u16`) |
| 2 | 1 | Kind: `0` file, `1` directory |
| 3 | 1 | Reserved, must be zero |
| 4 | 4 | Unix permission bits (`u32`) |
| 8 | 8 | Modification time, seconds since Unix epoch (`u64`) |
| 16 | 8 | Uncompressed size (`u64`) |
| 24 | 8 | Compressed size (`u64`) |
| 32 | 4 | CRC32 of uncompressed data (`u32`) |
| 36 | path length | UTF-8 relative path using `/` separators |
| after path | compressed size | Compressed file data |

Directory entries always have zero uncompressed size, compressed size, and
CRC32. File data is compressed with the algorithm named by the header.

## Huffman Compressed Data Format

The version 1 archive layout is unchanged; the Huffman payload is self-contained
within the entry's compressed data field. Multi-byte integers inside the Huffman
stream are little-endian.

- **Empty input**: Compresses to 0 bytes.
- **Fixed Header (10 bytes)**:
  - `0..8` (8 bytes): Uncompressed length in bytes (`u64`).
  - `8..10` (2 bytes): Number of distinct symbols present (`u16`, range `1..=256`).
- **Single-Symbol Stream (`symbol_count == 1`)**:
  - `10` (1 byte): The symbol byte (`u8`).
  - Total payload length is exactly 11 bytes. No code table or bitstream is stored.
- **Multi-Symbol Stream (`symbol_count >= 2`)**:
  - **Code Table (`symbol_count * 2` bytes)**: Starts at offset 10. Stores
    `symbol_count` pairs of `(symbol: u8, code_length: u8)`. Code lengths must
    be in range `1..=64`, symbols must be unique, and the lengths must satisfy
    the Kraft-McMillan equality ($\sum 2^{-L} = 1$). Canonical codes are
    assigned after sorting pairs by `(code_length, symbol)` ascending.
  - **Bitstream**: Begins at offset `10 + symbol_count * 2` and extends to the end
    of the entry's compressed data. Bits are packed MSB-first: for each symbol's
    canonical code, the most significant bit is written first into bit 7 of the
    byte, descending to bit 0. Unused bits in the final byte are padded with zero
    bits.
  - The reader stops decoding once the declared uncompressed length of symbols is
    reached, ensuring zero-padding is never interpreted as a symbol. Trailing
    bytes or non-zero padding bits are rejected as corrupt.

## Path and Size Rules

Paths must be relative, non-empty, valid UTF-8, and must not contain `..`,
absolute prefixes, NUL bytes, backslashes, or unsafe Windows device names.
Duplicate paths and file/directory conflicts are rejected. Symbolic links are
not stored by the filesystem compressor.

The reader limits paths to 4096 bytes, entries to 10,000,000, and an
individual uncompressed file to 1 TiB. These limits prevent malformed archives
from causing unbounded allocation or extraction outside the destination.

## Compatibility

- Existing RLE archives (algorithm ID `1`) remain completely unaffected and
  extract byte-for-byte identically.
- Archives created with Huffman coding (algorithm ID `2`) require a reader with
  Huffman support in its registry. Older readers or custom registries that lack ID `2`
  cleanly reject the archive with `Error::UnknownAlgorithm(2)`.
- The version 1 archive header and entry record layout are unchanged. Adding
  the Huffman algorithm required no changes to the container format.
