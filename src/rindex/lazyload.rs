//! Compatibility adapter over the shared bounded lazy-load readers.
//!
//! `rd-rds` owns index parsing and record decompression. Code records still
//! use Arity's object decoder because signature help needs default expressions
//! that the shared formal-inspection API does not yet expose.

use std::path::Path;

use rd_rds::lazyload::{self, LazyLoadIndex};

use crate::rindex::rds::{self, RdsError, Robj};

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

impl From<lazyload::Error> for LazyLoadError {
    fn from(error: lazyload::Error) -> Self {
        match error {
            lazyload::Error::Io { .. } => Self::Io(error.to_string()),
            _ => Self::BadIndex(error.to_string()),
        }
    }
}

/// Read sorted, unique object names without opening the companion `.rdb`.
pub fn read_index_names(rdx_path: &Path) -> Result<Vec<String>> {
    let index = LazyLoadIndex::open(rdx_path)?;
    Ok(sorted_names(index.variables()))
}

fn sorted_names(variables: &[lazyload::Variable]) -> Vec<String> {
    let mut names: Vec<_> = variables.iter().map(|v| v.name().to_owned()).collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// A shared lazy-load database with Arity's existing object-decoding API.
///
/// Records are read on demand. A changed or corrupt file fails the selected
/// fetch; callers can retain metadata and continue with other bindings.
pub struct LazyLoadDb {
    inner: lazyload::LazyLoadDb,
    names: Vec<String>,
}

impl LazyLoadDb {
    /// Open a database with a sibling `.rdb` having the same stem.
    pub fn open(rdx_path: &Path) -> Result<Self> {
        Self::open_pair(rdx_path, &rdx_path.with_extension("rdb"))
    }

    pub fn open_pair(rdx_path: &Path, rdb_path: &Path) -> Result<Self> {
        let inner = lazyload::LazyLoadDb::open(rdx_path, rdb_path)?;
        let names = sorted_names(inner.variables());
        Ok(Self { inner, names })
    }

    /// All object names, sorted and unique.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(String::as_str)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.inner.variable(name).is_some()
    }

    /// Fetch an object through the legacy decoder, retaining default expressions.
    pub fn fetch(&self, name: &str) -> Result<Robj> {
        let record = self.inner.read(name)?;
        Ok(rds::read_rds_stream(record.decompressed_bytes())?)
    }

    pub(crate) fn fetch_shared(&self, name: &str) -> Result<rd_rds::RObject> {
        let record = self.inner.read(name)?;
        rd_rds::parse(record.decompressed_bytes())
            .map_err(|error| LazyLoadError::BadIndex(error.to_string()))
    }
}
