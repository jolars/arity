//! Decode R's lazy-load databases (`.rdb` + `.rdx`).
//!
//! An installed package keeps its code in `R/{pkg}.rdb` (a concatenation of
//! individually-compressed serialized objects) indexed by `R/{pkg}.rdx`, and
//! its help likewise in `help/{pkg}.{rdb,rdx}`. The `.rdx` is itself an RDS
//! file holding a `variables` list mapping each object name to an
//! `[offset, length]` slice of the `.rdb`. Each slice is `R_compress1`-encoded:
//! a 4-byte big-endian uncompressed length followed by a zlib stream, which
//! decompresses to an ordinary RDS object stream.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use crate::rindex::rds::{self, RdsError, Rkind, Robj};

#[derive(Debug)]
pub enum LazyLoadError {
    Io(String),
    Rds(RdsError),
    BadIndex(String),
}

impl std::fmt::Display for LazyLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LazyLoadError::Io(s) => write!(f, "lazy-load I/O error: {s}"),
            LazyLoadError::Rds(e) => write!(f, "lazy-load RDS error: {e}"),
            LazyLoadError::BadIndex(s) => write!(f, "malformed .rdx index: {s}"),
        }
    }
}

impl std::error::Error for LazyLoadError {}

impl From<RdsError> for LazyLoadError {
    fn from(e: RdsError) -> Self {
        LazyLoadError::Rds(e)
    }
}

type Result<T> = std::result::Result<T, LazyLoadError>;

/// Read just the object names from a `.rdx` index, without touching the
/// (potentially large) `.rdb`. This is all the cheap harvest tier needs to
/// expand `exportPattern` directives.
pub fn read_index_names(rdx_path: &Path) -> Result<Vec<String>> {
    let rdx_bytes = std::fs::read(rdx_path).map_err(|e| LazyLoadError::Io(e.to_string()))?;
    let rdx = rds::read_rds(&rdx_bytes)?;
    let index = parse_index(&rdx)?;
    Ok(index.into_keys().collect())
}

/// A loaded lazy-load database: the `.rdb` bytes plus the name → slice index.
pub struct LazyLoadDb {
    rdb: Vec<u8>,
    /// Object name → (offset, length) into `rdb`, sorted by name.
    index: BTreeMap<String, (usize, usize)>,
}

impl LazyLoadDb {
    /// Open a database given the path to its `.rdx` index; the `.rdb` is
    /// expected alongside it with the same stem.
    pub fn open(rdx_path: &Path) -> Result<Self> {
        let rdb_path = rdx_path.with_extension("rdb");
        Self::open_pair(rdx_path, &rdb_path)
    }

    pub fn open_pair(rdx_path: &Path, rdb_path: &Path) -> Result<Self> {
        let rdx_bytes = std::fs::read(rdx_path).map_err(|e| LazyLoadError::Io(e.to_string()))?;
        let rdb = std::fs::read(rdb_path).map_err(|e| LazyLoadError::Io(e.to_string()))?;
        let rdx = rds::read_rds(&rdx_bytes)?;
        let index = parse_index(&rdx)?;
        Ok(LazyLoadDb { rdb, index })
    }

    /// All object names in the database, sorted.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.index.keys().map(|s| s.as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    /// Fetch and deserialize a single named object.
    pub fn fetch(&self, name: &str) -> Result<Robj> {
        let &(offset, length) = self
            .index
            .get(name)
            .ok_or_else(|| LazyLoadError::BadIndex(format!("no such object {name}")))?;
        let blob = self
            .rdb
            .get(offset..offset + length)
            .ok_or_else(|| LazyLoadError::BadIndex(format!("slice out of range for {name}")))?;
        let raw = decompress_blob(blob)?;
        Ok(rds::read_rds_stream(&raw)?)
    }
}

/// Decompress an `R_compress1` blob: 4-byte big-endian uncompressed length +
/// zlib stream.
fn decompress_blob(blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() < 4 {
        return Err(LazyLoadError::BadIndex(
            "blob shorter than length prefix".into(),
        ));
    }
    let ulen = u32::from_be_bytes([blob[0], blob[1], blob[2], blob[3]]) as usize;
    let mut decoder = flate2::read::ZlibDecoder::new(&blob[4..]);
    let mut out = Vec::with_capacity(ulen);
    decoder
        .read_to_end(&mut out)
        .map_err(|e| LazyLoadError::Io(e.to_string()))?;
    Ok(out)
}

/// Pull the `variables` map out of a parsed `.rdx` object.
fn parse_index(rdx: &Robj) -> Result<BTreeMap<String, (usize, usize)>> {
    let top_names = rdx
        .names()
        .ok_or_else(|| LazyLoadError::BadIndex("top level has no names".into()))?;
    let elems = rdx
        .as_list()
        .ok_or_else(|| LazyLoadError::BadIndex("top level is not a list".into()))?;
    let vars_idx = top_names
        .iter()
        .position(|n| *n == Some("variables"))
        .ok_or_else(|| LazyLoadError::BadIndex("no `variables` entry".into()))?;
    let vars = &elems[vars_idx];

    let var_names = vars
        .names()
        .ok_or_else(|| LazyLoadError::BadIndex("`variables` has no names".into()))?;
    let var_elems = match &vars.kind {
        Rkind::List(v) => v,
        _ => return Err(LazyLoadError::BadIndex("`variables` is not a list".into())),
    };

    let mut index = BTreeMap::new();
    for (name, slot) in var_names.iter().zip(var_elems.iter()) {
        let Some(name) = name else { continue };
        let pair = slot
            .as_int_vec()
            .ok_or_else(|| LazyLoadError::BadIndex(format!("{name}: not an int pair")))?;
        if pair.len() != 2 {
            return Err(LazyLoadError::BadIndex(format!(
                "{name}: pair len {}",
                pair.len()
            )));
        }
        let offset = pair[0].flatten_usize();
        let length = pair[1].flatten_usize();
        index.insert(name.to_string(), (offset, length));
    }
    Ok(index)
}

trait FlattenUsize {
    fn flatten_usize(self) -> usize;
}
impl FlattenUsize for Option<i32> {
    fn flatten_usize(self) -> usize {
        self.unwrap_or(0).max(0) as usize
    }
}
