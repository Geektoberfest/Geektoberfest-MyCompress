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

The built-in RLE algorithm has ID `1`.

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

## Path and Size Rules

Paths must be relative, non-empty, valid UTF-8, and must not contain `..`,
absolute prefixes, NUL bytes, backslashes, or unsafe Windows device names.
Duplicate paths and file/directory conflicts are rejected. Symbolic links are
not stored by the filesystem compressor.

The reader limits paths to 4096 bytes, entries to 10,000,000, and an
individual uncompressed file to 1 TiB. These limits prevent malformed archives
from causing unbounded allocation or extraction outside the destination.

## Compatibility

The format version and algorithm ID are explicit so readers can reject
archives they do not understand. Adding a new algorithm requires assigning a
new ID and updating the registry; changing the record layout requires a new
format version.
