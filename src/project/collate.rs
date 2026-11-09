//! The disk inventory used to validate package Collate specifications.
//!
//! Keep this separate from the graph's expected source set: that set unions
//! Collate entries in, including nonexistent files. Rules receive this snapshot
//! from the driver and never read the filesystem themselves.

use std::collections::BTreeSet;
use std::path::Path;

/// Eligible paths relative to `R/`, independently inventoried for each OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSourceInventory {
    pub unix: BTreeSet<String>,
    pub windows: BTreeSet<String>,
}

impl PackageSourceInventory {
    /// Match `tools::list_files_with_type(..., "code")` for both platforms.
    /// An unreadable or absent `R/` has no usable inventory.
    pub fn read(root: &Path) -> Option<Self> {
        let code = root.join("R");
        let common = eligible_names(&code)?;
        let platform = |os: &str| {
            let directory = code.join(os);
            let names = match std::fs::metadata(&directory) {
                Ok(metadata) if metadata.is_dir() => eligible_names(&directory)?,
                Ok(_) => BTreeSet::new(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
                Err(_) => return None,
            };
            Some(
                common
                    .iter()
                    .cloned()
                    .chain(names.into_iter().map(|name| format!("{os}/{name}")))
                    .collect(),
            )
        };
        Some(Self {
            unix: platform("unix")?,
            windows: platform("windows")?,
        })
    }
}

fn eligible_names(directory: &Path) -> Option<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for entry in std::fs::read_dir(directory).ok()? {
        let name = entry.ok()?.file_name().into_string().ok()?;
        // R's tools helper filters basenames as well as extensions and does
        // not recurse into arbitrary subdirectories. Uppercase Q is excluded.
        if name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
            && name
                .rsplit_once('.')
                .is_some_and(|(_, ext)| matches!(ext, "R" | "r" | "S" | "s" | "q"))
        {
            names.insert(name);
        }
    }
    Some(names)
}
