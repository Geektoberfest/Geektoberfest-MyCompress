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
mycompress compress input -a auto
```

### Automatic Algorithm Selection (`auto`)

When `-a auto` or `--algorithm auto` is specified, MyCompress evaluates a bounded sample of the input data to automatically choose the best available algorithm before creating the archive:

- **Bounded Sampling**: Samples up to 64 KiB of input data. For single files, the first 64 KiB is read. For directories, regular files are sampled cumulatively up to 64 KiB.
- **Evaluation Strategy**: Each registered algorithm in the registry is tested against the sample. An algorithm is only considered viable if its compressed output is strictly smaller than the uncompressed sample size.
- **Winner Selection**: The candidate yielding the smallest compressed size is chosen and its concrete algorithm ID is written to the archive header.
- **Deterministic Tie-Breaking & Fallback**:
  - If multiple algorithms produce identical compressed lengths, the algorithm with the lower numeric ID wins (preferring RLE over Huffman among built-ins).
  - If the input is empty or if no algorithm achieves compression (such as on pseudo-random or pre-compressed data), the compressor safely falls back to RLE (which has a strictly bounded worst-case expansion of ~0.8%).
- **Limitations**: A 64 KiB sample is a fast heuristic, not a guarantee of global optimality across large files or complex directories with heterogeneous file types.
- **Archive Compatibility**: Archives created with `auto` never store "auto" in the header; they record the concrete algorithm ID (`1` for RLE, `2` for Huffman). Existing archive readers and extractors handle them without any modifications.

The algorithm registry is extensible for library users. Custom algorithms
must use IDs `1024` or greater. IDs below `1024` are reserved for built-in
algorithms and future format-defined values.

Every decompressor must reject malformed input and return an error rather than
silently producing partial data. Archive extraction also checks the resulting
size and CRC32 after decompression.
