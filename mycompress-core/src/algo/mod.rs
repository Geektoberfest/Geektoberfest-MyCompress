//! Compression algorithm abstraction and registry.
//!
//! Every algorithm implements [`CompressionAlgorithm`] and is identified by a
//! numeric id (stored inside `.mcz` archives) and a short name (used on the
//! command line). Algorithms are looked up through the [`Registry`], so new
//! algorithms can be added without touching the archive layer or the CLI.
//!
//! Ids `1..1024` are reserved for built-in algorithms; third-party code may
//! register algorithms with ids `>= 1024`.

mod huffman;
mod rle;

pub use huffman::Huffman;
pub use rle::Rle;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use crate::error::{Error, Result};

/// Lowest id that third-party algorithms may claim.
pub const FIRST_CUSTOM_ID: u16 = 1024;

/// Identifies a compression algorithm inside a `.mcz` archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AlgorithmId {
    /// Run-Length Encoding (format id 1).
    Rle,
    /// Huffman coding (format id 2).
    Huffman,
    /// A third-party algorithm registered at runtime.
    Custom(u16),
}

impl AlgorithmId {
    /// The numeric id stored in archive headers.
    pub const fn as_u16(self) -> u16 {
        match self {
            Self::Rle => 1,
            Self::Huffman => 2,
            Self::Custom(id) => id,
        }
    }

    /// Resolves the ids defined by the `.mcz` format. Reserved-but-unimplemented
    /// built-in ids resolve to `None`.
    pub fn from_u16(id: u16) -> Option<Self> {
        match id {
            1 => Some(Self::Rle),
            2 => Some(Self::Huffman),
            _ if id >= FIRST_CUSTOM_ID => Some(Self::Custom(id)),
            _ => None,
        }
    }
}

impl fmt::Display for AlgorithmId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rle => write!(f, "rle"),
            Self::Huffman => write!(f, "huffman"),
            Self::Custom(id) => write!(f, "custom-{id}"),
        }
    }
}

/// A compression algorithm: a pair of compress/decompress functions.
///
/// Implementations must round-trip every input exactly:
/// `decompress(compress(data)) == data`. Implementations must be pure with
/// respect to their input and must never panic on malformed data; corrupt
/// input is reported as an [`Error`](crate::Error) instead.
pub trait CompressionAlgorithm: Send + Sync {
    /// The algorithm id stored in archives.
    fn id(&self) -> AlgorithmId;
    /// Short, stable, lower-case name used on the command line.
    fn name(&self) -> &'static str;
    /// Compresses `data`.
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>>;
    /// Decompresses `data` produced by [`compress`](CompressionAlgorithm::compress).
    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>>;
}

/// Maps algorithm ids to implementations.
///
/// The registry is the only place where concrete algorithms are known; the
/// archive reader and the CLI look algorithms up by id or name. Constructed
/// with all built-ins via [`Registry::new`], or empty via
/// [`Registry::default`] for fully custom setups.
#[derive(Clone, Default)]
pub struct Registry {
    algorithms: HashMap<u16, Arc<dyn CompressionAlgorithm>>,
}

impl Registry {
    /// A registry containing all built-in algorithms.
    pub fn new() -> Self {
        let mut registry = Self::default();
        registry
            .register(Arc::new(Rle))
            .expect("built-in algorithm ids are unique");
        registry
            .register(Arc::new(Huffman))
            .expect("built-in algorithm ids are unique");
        registry
    }

    /// Registers an algorithm. Fails for reserved ids and duplicate ids.
    pub fn register(&mut self, algorithm: Arc<dyn CompressionAlgorithm>) -> Result<()> {
        match algorithm.id() {
            AlgorithmId::Rle | AlgorithmId::Huffman => {}
            AlgorithmId::Custom(id) if id >= FIRST_CUSTOM_ID => {}
            AlgorithmId::Custom(id) => return Err(Error::ReservedAlgorithmId(id)),
        }
        let key = algorithm.id().as_u16();
        if self.algorithms.contains_key(&key) {
            return Err(Error::InvalidInput(format!(
                "algorithm id {key} is already registered"
            )));
        }
        self.algorithms.insert(key, algorithm);
        Ok(())
    }

    /// Looks up an algorithm by numeric id.
    pub fn get(&self, id: u16) -> Option<Arc<dyn CompressionAlgorithm>> {
        self.algorithms.get(&id).cloned()
    }

    /// Looks up an algorithm by name (case-insensitive).
    pub fn get_by_name(&self, name: &str) -> Option<Arc<dyn CompressionAlgorithm>> {
        self.algorithms
            .values()
            .find(|a| a.name().eq_ignore_ascii_case(name))
            .cloned()
    }

    /// Sorted list of registered algorithm names.
    pub fn names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.algorithms.values().map(|a| a.name()).collect();
        names.sort_unstable();
        names
    }

    /// All registered algorithms, sorted deterministically by algorithm ID.
    pub fn all(&self) -> Vec<Arc<dyn CompressionAlgorithm>> {
        let mut algos: Vec<_> = self.algorithms.values().cloned().collect();
        algos.sort_by_key(|a| a.id());
        algos
    }

    /// Evaluates registered algorithms on a sample of input data and selects
    /// the algorithm that produces the smallest compressed output.
    ///
    /// See [`select_algorithm`] for strategy and tie-breaking details.
    pub fn select_algorithm(&self, sample: &[u8]) -> AlgorithmId {
        select_algorithm(sample, self)
    }
}

/// Evaluates registered algorithms on a sample of input data and selects
/// the algorithm that produces the smallest compressed output.
///
/// # Selection Strategy
///
/// 1. If `sample` is empty, the fallback algorithm is returned (preferring [`AlgorithmId::Rle`]).
/// 2. Each registered algorithm in `registry` is evaluated by calling its
///    [`compress`](CompressionAlgorithm::compress) method on `sample`.
/// 3. Candidates that return an error during trial compression or whose compressed
///    output is not strictly smaller than `sample.len()` are disqualified.
/// 4. Among valid candidates, the algorithm with the smallest compressed length wins.
/// 5. Ties are resolved deterministically by choosing the candidate with the lowest
///    [`AlgorithmId`] (preferring RLE over Huffman for built-in algorithms).
/// 6. If no candidate reduces the sample (such as on incompressible or pseudo-random data,
///    or when all candidates error), the fallback algorithm ([`AlgorithmId::Rle`] if present,
///    or the first available registered algorithm) is returned.
pub fn select_algorithm(sample: &[u8], registry: &Registry) -> AlgorithmId {
    let mut candidates = registry.all();
    candidates.sort_by_key(|a| a.id());

    let fallback = candidates
        .iter()
        .map(|a| a.id())
        .find(|&id| id == AlgorithmId::Rle)
        .or_else(|| candidates.first().map(|a| a.id()))
        .unwrap_or(AlgorithmId::Rle);

    if sample.is_empty() {
        return fallback;
    }

    let mut best_id = None;
    let mut best_size = usize::MAX;

    for algo in candidates {
        if let Ok(compressed) = algo.compress(sample) {
            if compressed.len() < sample.len() && compressed.len() < best_size {
                best_size = compressed.len();
                best_id = Some(algo.id());
            }
        }
    }

    best_id.unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A toy pass-through algorithm used to exercise the registry.
    struct Passthrough(u16);

    impl CompressionAlgorithm for Passthrough {
        fn id(&self) -> AlgorithmId {
            AlgorithmId::Custom(self.0)
        }
        fn name(&self) -> &'static str {
            "passthrough"
        }
        fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
            Ok(data.to_vec())
        }
        fn decompress(&self, data: &[u8]) -> Result<Vec<u8>> {
            Ok(data.to_vec())
        }
    }

    /// A mock algorithm that shrinks data to a fixed length or expands.
    struct MockShrink(u16, usize);

    impl CompressionAlgorithm for MockShrink {
        fn id(&self) -> AlgorithmId {
            AlgorithmId::Custom(self.0)
        }
        fn name(&self) -> &'static str {
            "mock-shrink"
        }
        fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
            if self.1 < data.len() {
                Ok(vec![0u8; self.1])
            } else {
                Ok(vec![0u8; data.len() + 10])
            }
        }
        fn decompress(&self, _data: &[u8]) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
    }

    /// A mock algorithm that always expands data.
    struct MockExpand(u16);

    impl CompressionAlgorithm for MockExpand {
        fn id(&self) -> AlgorithmId {
            AlgorithmId::Custom(self.0)
        }
        fn name(&self) -> &'static str {
            "mock-expand"
        }
        fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
            let mut out = data.to_vec();
            out.push(0);
            Ok(out)
        }
        fn decompress(&self, _data: &[u8]) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
    }

    /// A mock algorithm that always fails during compression.
    struct MockError(u16);

    impl CompressionAlgorithm for MockError {
        fn id(&self) -> AlgorithmId {
            AlgorithmId::Custom(self.0)
        }
        fn name(&self) -> &'static str {
            "mock-error"
        }
        fn compress(&self, _data: &[u8]) -> Result<Vec<u8>> {
            Err(Error::CorruptArchive("compression failed".to_string()))
        }
        fn decompress(&self, _data: &[u8]) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn builtin_registry_contains_builtins() {
        let registry = Registry::new();
        assert!(registry.get(1).is_some());
        assert!(registry.get_by_name("rle").is_some());
        assert!(registry.get_by_name("RLE").is_some());
        assert!(registry.get(2).is_some());
        assert!(registry.get_by_name("huffman").is_some());
        assert!(registry.get_by_name("HUFFMAN").is_some());
        assert_eq!(registry.names(), vec!["huffman", "rle"]);
    }

    #[test]
    fn custom_algorithms_can_be_registered() {
        let mut registry = Registry::new();
        registry.register(Arc::new(Passthrough(2000))).unwrap();
        assert!(registry.get(2000).is_some());
        assert!(registry.get_by_name("passthrough").is_some());

        // Duplicate ids are rejected.
        assert!(registry.register(Arc::new(Passthrough(2000))).is_err());
        // Reserved ids cannot be claimed by custom algorithms.
        assert!(matches!(
            registry.register(Arc::new(Passthrough(1))),
            Err(Error::ReservedAlgorithmId(1))
        ));
        assert!(matches!(
            registry.register(Arc::new(Passthrough(2))),
            Err(Error::ReservedAlgorithmId(2))
        ));
        assert!(matches!(
            registry.register(Arc::new(Passthrough(500))),
            Err(Error::ReservedAlgorithmId(500))
        ));
        assert!(registry.get(999).is_none());
    }

    #[test]
    fn select_algorithm_empty_sample_returns_fallback() {
        let registry = Registry::new();
        assert_eq!(select_algorithm(&[], &registry), AlgorithmId::Rle);
        assert_eq!(registry.select_algorithm(&[]), AlgorithmId::Rle);
    }

    #[test]
    fn select_algorithm_text_prefers_huffman() {
        let registry = Registry::new();
        let sample = b"The quick brown fox jumps over the lazy dog. Huffman coding produces optimal prefix codes for known symbol frequencies, whereas run-length encoding excels on consecutive repetitions. ";
        let mut text = Vec::new();
        while text.len() < 1_000 {
            text.extend_from_slice(sample);
        }
        text.truncate(1_000);
        let chosen = select_algorithm(&text, &registry);
        assert_eq!(chosen, AlgorithmId::Huffman);
    }

    #[test]
    fn select_algorithm_repetitive_data_selects_compressor() {
        let registry = Registry::new();
        // Alternating runs of repeated bytes (excels with RLE)
        let mut repetitive = Vec::new();
        for _ in 0..10 {
            repetitive.extend_from_slice(&[b'A'; 50]);
            repetitive.extend_from_slice(&[b'B'; 50]);
        }
        let chosen = select_algorithm(&repetitive, &registry);
        // Either RLE or Huffman compresses this well, and select_algorithm picks a compressor
        assert!(matches!(chosen, AlgorithmId::Rle | AlgorithmId::Huffman));
    }

    #[test]
    fn select_algorithm_incompressible_falls_back_to_rle() {
        let registry = Registry::new();
        // High entropy sequence with no repeated bytes and uniform distribution.
        let mut random_like = Vec::with_capacity(256);
        for b in 0u8..=255 {
            random_like.push(b);
        }
        let chosen = select_algorithm(&random_like, &registry);
        assert_eq!(chosen, AlgorithmId::Rle);
    }

    #[test]
    fn select_algorithm_tie_breaking_favors_lower_id() {
        let mut registry = Registry::default();
        // Register two mock algorithms that produce identical output size (10 bytes).
        registry.register(Arc::new(MockShrink(1050, 10))).unwrap();
        registry.register(Arc::new(MockShrink(1040, 10))).unwrap();

        let sample = vec![1u8; 100];
        let chosen = select_algorithm(&sample, &registry);
        // ID 1040 is lower than 1050, so it must win deterministically.
        assert_eq!(chosen, AlgorithmId::Custom(1040));
    }

    #[test]
    fn select_algorithm_ignores_algorithms_that_expand() {
        let mut registry = Registry::default();
        // Register an expanding algorithm with lower ID (2000)
        // and a shrinking algorithm with higher ID (2001).
        registry.register(Arc::new(MockExpand(2000))).unwrap();
        registry.register(Arc::new(MockShrink(2001, 10))).unwrap();

        let sample = vec![0u8; 100];
        let chosen = select_algorithm(&sample, &registry);
        // MockExpand(2000) has lower ID but expanded, so MockShrink(2001) must win.
        assert_eq!(chosen, AlgorithmId::Custom(2001));
    }

    #[test]
    fn select_algorithm_disqualifies_algorithms_that_error() {
        let mut registry = Registry::default();
        // MockError has lower ID (2000); MockShrink has higher ID (2001).
        registry.register(Arc::new(MockError(2000))).unwrap();
        registry.register(Arc::new(MockShrink(2001, 10))).unwrap();

        let sample = vec![0u8; 100];
        let chosen = select_algorithm(&sample, &registry);
        assert_eq!(chosen, AlgorithmId::Custom(2001));
    }

    #[test]
    fn select_algorithm_fallback_when_no_rle_or_all_error() {
        let mut registry = Registry::default();
        registry.register(Arc::new(MockError(2000))).unwrap();
        registry.register(Arc::new(MockError(2001))).unwrap();

        let sample = vec![0u8; 100];
        // When all algorithms error and there is no RLE, fallback is the first registered algorithm.
        let chosen = select_algorithm(&sample, &registry);
        assert_eq!(chosen, AlgorithmId::Custom(2000));
    }
}
