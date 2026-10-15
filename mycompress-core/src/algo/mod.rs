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

/// Hard cap on bytes a single decompress call may produce (64 MiB).
/// Decoders must reject headers claiming more than this before allocating.
pub const MAX_DECOMPRESSED_BYTES: u64 = 64 << 20;

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
}
