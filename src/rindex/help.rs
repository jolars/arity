//! The installed help index and a cache of the Rd topics requested from it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use rd_helpdb::{HelpTopicIndex, HelpTopicText, PackageHelpDb};
use smol_str::SmolStr;

use super::rd::{self, RdSections};
use super::schema::{HelpAssociation, HelpDoc};

#[derive(Debug)]
pub(super) struct HelpFiles {
    index: HelpTopicIndex,
    package_dir: PathBuf,
    package: SmolStr,
    database: OnceLock<Option<HelpDatabase>>,
    topics: Mutex<HashMap<SmolStr, Arc<OnceLock<RdSections>>>>,
}

impl HelpFiles {
    /// Opening the metadata gives callers the alias map without touching the
    /// compiled help database.
    pub(super) fn open(package_dir: &Path, package: &str) -> Option<Self> {
        Some(Self {
            index: read_help_index(package_dir)?,
            package_dir: package_dir.to_owned(),
            package: package.into(),
            database: OnceLock::new(),
            topics: Mutex::new(HashMap::new()),
        })
    }

    pub(super) fn association(&self, alias: &str) -> Option<HelpAssociation> {
        let entry = self.index.find_alias(alias)?;
        Some(HelpAssociation {
            alias: alias.into(),
            topic: entry.topic_key().map(Into::into),
        })
    }

    pub(super) fn document(&self, alias: &str) -> Option<HelpDoc> {
        let entry = self.index.find_alias(alias)?;
        let sections = entry
            .topic_key()
            .map(|key| {
                // Hold the map lock only to get the cell. Requests for other
                // topics may decode in parallel while this cell is filling.
                let topic = {
                    let mut topics = self.topics.lock().unwrap();
                    if let Some(topic) = topics.get(key) {
                        Arc::clone(topic)
                    } else {
                        let topic = Arc::new(OnceLock::new());
                        topics.insert(key.into(), Arc::clone(&topic));
                        topic
                    }
                };
                topic.get_or_init(|| self.render_topic(key)).clone()
            })
            .unwrap_or_default();
        let doc = rd::into_help_doc(entry.title.as_str().map(str::to_owned), sections);
        (doc != HelpDoc::default()).then_some(doc)
    }

    fn render_topic(&self, key: &str) -> RdSections {
        let database = self
            .database
            .get_or_init(|| {
                HelpDatabase::open(&self.package_dir, &self.package)
                    .inspect_err(|error| {
                        log::debug!(
                            "{}: compiled help unavailable: {error}",
                            self.package_dir.display()
                        );
                    })
                    .ok()
            })
            .as_ref();
        let Some(database) = database else {
            return RdSections::default();
        };
        match database.document(key) {
            Ok(document) => {
                let rendered = rd::render_document(&document);
                for issue in rendered.issues {
                    log::debug!(
                        "{}: topic {key:?}, omitted {} at {:?}",
                        self.package_dir.display(),
                        issue.field,
                        issue.path
                    );
                }
                rendered.sections
            }
            Err(error) => {
                log::debug!(
                    "{}: topic {key:?} unavailable: {error}",
                    self.package_dir.display()
                );
                RdSections::default()
            }
        }
    }
}

fn read_help_index(pkg_dir: &Path) -> Option<HelpTopicIndex> {
    match HelpTopicIndex::read_installed(pkg_dir) {
        Ok(index) => {
            if let Some(index) = &index {
                for (row, entry) in index.entries().enumerate() {
                    for (field, value) in [("title", &entry.title), ("file", &entry.file)] {
                        if let HelpTopicText::Invalid(reason) = value {
                            log::debug!(
                                "{}: help metadata row {row} {field}: {reason}",
                                pkg_dir.display()
                            );
                        }
                    }
                }
            }
            index
        }
        Err(error) => {
            log::debug!("{}: help metadata unavailable: {error}", pkg_dir.display());
            None
        }
    }
}

#[derive(Debug)]
enum HelpDatabase {
    Package(PackageHelpDb),
    // The public harvester also accepts renamed directories. The package view
    // assumes basename == Package, so those paths need explicit shared access.
    Explicit(rd_rds::lazyload::LazyLoadDb),
}

impl HelpDatabase {
    fn open(package_dir: &Path, package: &str) -> std::result::Result<Self, String> {
        if package_dir.file_name().and_then(|s| s.to_str()) == Some(package) {
            PackageHelpDb::open(package_dir)
                .map(Self::Package)
                .map_err(|e| e.to_string())
        } else {
            let help = package_dir.join("help");
            rd_rds::lazyload::LazyLoadDb::open(
                help.join(format!("{package}.rdx")),
                help.join(format!("{package}.rdb")),
            )
            .map(Self::Explicit)
            .map_err(|e| e.to_string())
        }
    }

    fn document(&self, key: &str) -> std::result::Result<rd_ast::RdDocument, String> {
        let object = match self {
            Self::Package(db) => db.raw_topic(key).map_err(|e| e.to_string())?,
            Self::Explicit(db) => {
                let record = db.read(key).map_err(|e| e.to_string())?;
                rd_rds::parse(record.decompressed_bytes()).map_err(|e| e.to_string())?
            }
        };
        rd_ast::lower_r_object(&object).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_aliases_reuse_rendered_topic() {
        let directory =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rindex/oopfixture");
        let help = HelpFiles::open(&directory, "oopfixture").unwrap();
        assert!(help.database.get().is_none());
        assert!(help.association("Shape-class").is_some());
        assert!(help.database.get().is_none());

        let class = help.document("Shape-class").unwrap();
        let method = help.document("area,Shape-method").unwrap();
        assert!(!class.arguments.is_empty());
        assert_eq!(method, class);
        assert_eq!(help.topics.lock().unwrap().len(), 1);
    }
}
