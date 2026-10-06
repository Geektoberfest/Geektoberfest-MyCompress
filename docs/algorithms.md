# Compression Algorithms

MyCompress separates archive handling from compression algorithms. An
algorithm has a stable numeric ID, a display name, and `compress` and
`decompress` operations. The archive stores the numeric ID in its header so a
reader can select the correct implementation.

## RLE

RLE is the built-in algorithm and has ID `1`. It encodes repeated bytes as
repeat blocks and other bytes as literal blocks. Long runs are split into
multiple blocks when necessary, and incompressible input remains bounded in
size rather than requiring a large temporary representation.

RLE is a small, dependency-free format intended for predictable behavior and
fast operation. It is not a general-purpose replacement for modern formats
such as zstd or gzip; data without repeated bytes may become slightly larger.

## Choosing an Algorithm

The CLI currently supports the default algorithm only:

```text
mycompress compress input -a rle
```

The algorithm registry is extensible for library users. Custom algorithms
must use IDs `1024` or greater. IDs below `1024` are reserved for built-in
algorithms and future format-defined values.

Every decompressor must reject malformed input and return an error rather than
silently producing partial data. Archive extraction also checks the resulting
size and CRC32 after decompression.
