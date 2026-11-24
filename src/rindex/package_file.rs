//! Seekable local package cache. One atomic file keeps all views in sync.
//!
//! The header stores the schema version and four section lengths as little-endian
//! u32s. The export list, symbol directory, package metadata, and symbol array
//! are independent JSON sections. Directory ranges point into the symbol array.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use smol_str::SmolStr;

use super::cache::CacheError;
use super::schema::{ExportEntry, PackageExports, PackageIndex, SCHEMA_VERSION, SymbolEntry};

const HEADER_LEN: u64 = 20;
type Directory = BTreeMap<SmolStr, SymbolRecord>;

#[derive(Debug, Serialize, Deserialize)]
struct SymbolRecord(u32, u32, #[serde(skip)] OnceLock<Option<SymbolEntry>>);

/// An open handle pins a generation across atomic replacements. The lock
/// protects the shared seek cursor. Decoding takes place outside the lock.
#[derive(Debug)]
pub(super) struct PackageFile {
    file: Mutex<File>,
    export_len: u32,
    directory_len: u32,
    metadata_len: u32,
    symbols_len: u32,
    directory: OnceLock<Option<Directory>>,
}

impl PackageFile {
    pub(super) fn open(mut file: File) -> Option<Self> {
        let mut header = [0; HEADER_LEN as usize];
        file.read_exact(&mut header).ok()?;
        let values = [0, 4, 8, 12, 16]
            .map(|start| u32::from_le_bytes(header[start..start + 4].try_into().unwrap()));
        let [
            version,
            export_len,
            directory_len,
            metadata_len,
            symbols_len,
        ] = values;
        let expected_len = HEADER_LEN
            + u64::from(export_len)
            + u64::from(directory_len)
            + u64::from(metadata_len)
            + u64::from(symbols_len);
        if version != SCHEMA_VERSION || file.metadata().ok()?.len() != expected_len {
            return None;
        }
        Some(Self {
            file: Mutex::new(file),
            export_len,
            directory_len,
            metadata_len,
            symbols_len,
            directory: OnceLock::new(),
        })
    }

    fn read_section<T: serde::de::DeserializeOwned>(&self, start: u64, len: u32) -> Option<T> {
        let bytes = {
            let mut file = self.file.lock().ok()?;
            file.seek(SeekFrom::Start(start)).ok()?;
            let mut bytes = Vec::new();
            (&mut *file)
                .take(u64::from(len))
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() != len as usize {
                return None;
            }
            bytes
        };
        serde_json::from_slice(&bytes).ok()
    }

    fn metadata_start(&self) -> u64 {
        HEADER_LEN + u64::from(self.export_len) + u64::from(self.directory_len)
    }

    fn symbols_start(&self) -> u64 {
        self.metadata_start() + u64::from(self.metadata_len)
    }

    pub(super) fn exports(&self) -> Option<PackageExports> {
        let exports: PackageExports = self.read_section(HEADER_LEN, self.export_len)?;
        (exports.schema_version == SCHEMA_VERSION).then_some(exports)
    }

    pub(super) fn metadata(&self) -> Option<PackageIndex> {
        let index: PackageIndex = self.read_section(self.metadata_start(), self.metadata_len)?;
        (index.schema_version == SCHEMA_VERSION && index.symbols.is_empty()).then_some(index)
    }

    pub(super) fn package(&self) -> Option<PackageIndex> {
        let mut index = self.metadata()?;
        index.symbols = self.read_section(self.symbols_start(), self.symbols_len)?;
        Some(index)
    }

    pub(super) fn lookup(&self, name: &str) -> Option<&SymbolEntry> {
        let directory = self
            .directory
            .get_or_init(|| {
                self.read_section(HEADER_LEN + u64::from(self.export_len), self.directory_len)
            })
            .as_ref()?;
        let SymbolRecord(offset, len, cached) = directory.get(name)?;
        cached
            .get_or_init(|| {
                if offset.checked_add(*len)? > self.symbols_len {
                    return None;
                }
                let entry: SymbolEntry =
                    self.read_section(self.symbols_start() + u64::from(*offset), *len)?;
                (entry.name == name).then_some(entry)
            })
            .as_ref()
    }
}

pub(super) fn encode(index: &PackageIndex) -> Result<Vec<u8>, CacheError> {
    let serde_error = |error: serde_json::Error| CacheError::Serde(error.to_string());
    let size = |len| {
        u32::try_from(len)
            .map_err(|_| CacheError::Io("package index section exceeds 4 GiB".to_owned()))
    };
    let exports = PackageExports {
        schema_version: index.schema_version,
        package: index.package.clone(),
        version: index.version.clone(),
        attaches: index.attaches.clone(),
        symbols: index
            .symbols
            .iter()
            .map(|symbol| ExportEntry {
                name: symbol.name.clone(),
                exported: symbol.exported,
            })
            .collect(),
    };
    let exports = serde_json::to_vec(&exports).map_err(serde_error)?;
    let metadata = PackageIndex {
        schema_version: index.schema_version,
        package: index.package.clone(),
        version: index.version.clone(),
        lib_path: index.lib_path.clone(),
        title: index.title.clone(),
        r_version: index.r_version.clone(),
        harvested_at: index.harvested_at,
        attaches: index.attaches.clone(),
        symbols: Vec::new(),
        objects: index.objects.clone(),
    };
    let metadata = serde_json::to_vec(&metadata).map_err(serde_error)?;
    let symbols = serde_json::to_vec(&index.symbols).map_err(serde_error)?;
    let records: Vec<&RawValue> = serde_json::from_slice(&symbols).map_err(serde_error)?;
    let mut directory = BTreeMap::new();
    for (symbol, raw) in index.symbols.iter().zip(records) {
        // Borrowed RawValue points at the exact slice in the compact symbol array.
        let raw = raw.get();
        let start = size(raw.as_ptr() as usize - symbols.as_ptr() as usize)?;
        let len = size(raw.len())?;
        directory
            .entry(symbol.name.clone())
            .or_insert(SymbolRecord(start, len, OnceLock::new()));
    }
    let directory = serde_json::to_vec(&directory).map_err(serde_error)?;
    let header = [
        index.schema_version,
        size(exports.len())?,
        size(directory.len())?,
        size(metadata.len())?,
        size(symbols.len())?,
    ];
    let mut file = Vec::with_capacity(
        HEADER_LEN as usize + exports.len() + directory.len() + metadata.len() + symbols.len(),
    );
    for field in header {
        file.extend(field.to_le_bytes());
    }
    file.extend(exports);
    file.extend(directory);
    file.extend(metadata);
    file.extend(symbols);
    Ok(file)
}
