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

## Huffman

Huffman coding is a built-in algorithm with ID `2`. It generates canonical
variable-length prefix codes based on byte frequencies, assigning shorter bit
sequences to more frequent symbols.

Huffman coding is well suited for inputs with non-uniform symbol frequencies,
such as plain text, source code, or structured binary data, even when identical
bytes do not appear in consecutive runs.

Limitations:
- **Metadata overhead on tiny inputs**: storing uncompressed length and the
  code table requires a small header, which can exceed the data savings on very
  short inputs.
- **Uniformly distributed or incompressible data**: when byte frequencies are
  nearly uniform (or data is already compressed/encrypted), Huffman codes cannot
  reduce the size and may add minor overhead for the code table.

## Algorithm Comparison

Different algorithms suit different data characteristics. Neither algorithm
consistently produces smaller output across all inputs: RLE performs best on
long runs of identical bytes, while Huffman performs best when symbol
frequencies are skewed across the input.

The following table shows compressed sizes measured across representative inputs:

| Input | Original Size | RLE Size | Huffman Size |
| --- | ---: | ---: | ---: |
| Long single-symbol run (`'A'` x 10,000) | 10,000 bytes | 154 bytes | 11 bytes |
| English text sample | 1,000 bytes | 1,008 bytes | 636 bytes |
| Uniformly random bytes | 10,000 bytes | 10,079 bytes | 10,516 bytes |
| Tiny input (`"test"`) | 4 bytes | 5 bytes | 17 bytes |

## Choosing an Algorithm

The CLI supports selecting an algorithm via `--algorithm` / `-a` (defaults to `rle`):

```text
mycompress compress input -a rle
mycompress compress input -a huffman
```

You can also compare compression performance on your own files using the `benchmark` command: `mycompress benchmark <FILE>...`.

The algorithm registry is extensible for library users. Custom algorithms
must use IDs `1024` or greater. IDs below `1024` are reserved for built-in
algorithms and future format-defined values.

Every decompressor must reject malformed input and return an error rather than
silently producing partial data. Archive extraction also checks the resulting
size and CRC32 after decompression.
