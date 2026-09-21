//! Consumer contracts for the transition to the shared Rd readers.

use std::path::{Path, PathBuf};

use arity::rindex::harvest::{HarvestOptions, harvest_package};
use arity::rindex::lazyload::{LazyLoadDb, read_index_names};
use arity::rindex::rd;
use arity::rindex::schema::SymbolKind;

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rindex")
        .join(path)
}

fn copy_tree(source: &Path, dest: &Path) {
    std::fs::create_dir_all(dest).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dest.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
        }
    }
}

fn package_copy() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let pkg = temp.path().join("readerfixture");
    copy_tree(&fixture("migration/readerfixture"), &pkg);
    (temp, pkg)
}

#[test]
fn index_names_need_no_data_file() {
    for (path, count) in [
        ("magrittr/R/magrittr.rdx", 52),
        ("R.oo/R/R.oo.rdx", 349),
        ("metatoy/R/metatoy.rdx", 6),
        ("lazydata/data/Rdata.rdx", 2),
        ("magrittr/help/magrittr.rdx", 15),
    ] {
        let expected = read_index_names(&fixture(path)).unwrap();
        assert_eq!(expected.len(), count);
        let temp = tempfile::tempdir().unwrap();
        let index = temp.path().join("arbitrary.rdx");
        std::fs::copy(fixture(path), &index).unwrap();
        assert_eq!(read_index_names(&index).unwrap(), expected);
        assert!(expected.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(!index.with_extension("rdb").exists());
    }
}

#[test]
fn duplicate_names_are_sorted_unique_and_last_record_wins() {
    let index = fixture("migration/duplicates.rdx");
    assert_eq!(read_index_names(&index).unwrap(), ["a", "z"]);
    let db = LazyLoadDb::open(&index).unwrap();
    assert_eq!(db.names().collect::<Vec<_>>(), ["a", "z"]);
    assert_eq!(db.fetch("a").unwrap().as_int_vec().unwrap(), [Some(3)]);
    assert!(db.contains("z"));
    assert!(db.fetch("absent").is_err());
}

#[test]
fn invalid_offsets_are_rejected_instead_of_coerced() {
    for name in ["negative.rdx", "na-offset.rdx"] {
        assert!(read_index_names(&fixture(&format!("migration/{name}"))).is_err());
    }
}

#[test]
fn empty_index_is_valid() {
    assert!(
        read_index_names(&fixture("migration/empty.rdx"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn complete_magrittr_help_baseline() {
    let db = LazyLoadDb::open(&fixture("magrittr/help/magrittr.rdx")).unwrap();
    let pages: Vec<_> = db
        .names()
        .map(|name| {
            let sections = rd::render_page(&db.fetch(name).unwrap());
            (name, rd::into_help_doc(None, sections))
        })
        .collect();
    insta::assert_snapshot!(serde_json::to_string_pretty(&pages).unwrap());
}

#[test]
fn synthetic_harvest_preserves_kinds_defaults_and_data_collision() {
    let index = harvest_package(
        &fixture("migration/readerfixture"),
        HarvestOptions::default(),
        0,
    )
    .unwrap();
    let find = |name: &str| index.symbols.iter().find(|s| s.name == name).unwrap();
    assert_eq!(find("zero").formals.as_ref().unwrap().len(), 0);
    assert_eq!(find("primitive").kind, SymbolKind::Function);
    assert!(find("primitive").formals.is_none());
    assert_eq!(find("number").kind, SymbolKind::Data);
    assert_eq!(
        index.symbols.iter().filter(|s| s.name == "number").count(),
        1
    );
    assert_eq!(find("dataset").kind, SymbolKind::Data);
    let formals = find("defaults").formals.as_ref().unwrap();
    assert_eq!(
        formals
            .iter()
            .map(|f| f.default.as_deref())
            .collect::<Vec<_>>(),
        [
            None,
            Some("TRUE"),
            Some("1L"),
            Some("\"text\""),
            Some("NULL")
        ]
    );
    assert_eq!(
        find("zero").help.as_ref().unwrap().title.as_deref(),
        Some("Metadata title")
    );
}

#[test]
fn lazydata_harvesting_ignores_missing_data_records() {
    let (temp, pkg) = package_copy();
    let before = harvest_package(&pkg, HarvestOptions::default(), 0).unwrap();
    std::fs::remove_file(pkg.join("data/Rdata.rdb")).unwrap();
    assert_eq!(
        harvest_package(&pkg, HarvestOptions::default(), 0).unwrap(),
        before
    );
    drop(temp);
}

#[test]
fn renamed_package_directory_preserves_help_and_signatures() {
    let (temp, pkg) = package_copy();
    let before = harvest_package(&pkg, HarvestOptions::default(), 0).unwrap();
    let renamed = temp.path().join("different-directory");
    std::fs::rename(pkg, &renamed).unwrap();
    assert_eq!(
        harvest_package(&renamed, HarvestOptions::default(), 0).unwrap(),
        before
    );
}

#[test]
fn missing_metadata_does_not_recover_aliases_from_compiled_help() {
    let (_temp, pkg) = package_copy();
    std::fs::remove_file(pkg.join("Meta/Rd.rds")).unwrap();
    let index = harvest_package(&pkg, HarvestOptions::default(), 0).unwrap();
    assert!(index.symbols.iter().all(|s| s.help.is_none()));
}

#[test]
fn metadata_topic_keys_follow_r_basename_and_suffix_rules() {
    let (_temp, pkg) = package_copy();
    std::fs::copy(fixture("migration/normalized.rds"), pkg.join("Meta/Rd.rds")).unwrap();
    let index = harvest_package(&pkg, HarvestOptions::default(), 0).unwrap();
    let help = index
        .symbols
        .iter()
        .find(|s| s.name == "zero")
        .unwrap()
        .help
        .as_ref()
        .unwrap();
    assert_eq!(help.usage.as_deref(), Some("zero()"));
}

#[test]
fn optional_metadata_fields_recover_without_losing_other_rows() {
    for file in ["optional.rds", "invalid-title.rds"] {
        let (_temp, pkg) = package_copy();
        std::fs::copy(
            fixture(&format!("migration/{file}")),
            pkg.join("Meta/Rd.rds"),
        )
        .unwrap();
        let index = harvest_package(&pkg, HarvestOptions::default(), 0).unwrap();
        let other = index
            .symbols
            .iter()
            .find(|s| s.name == "number")
            .unwrap()
            .help
            .as_ref()
            .unwrap();
        assert_eq!(other.title.as_deref(), Some("Page title"));
        assert!(other.description.is_some());
    }
}

#[test]
fn malformed_alias_schema_does_not_partially_retarget_help() {
    let (_temp, pkg) = package_copy();
    std::fs::copy(
        fixture("migration/invalid-aliases.rds"),
        pkg.join("Meta/Rd.rds"),
    )
    .unwrap();
    let index = harvest_package(&pkg, HarvestOptions::default(), 0).unwrap();
    assert!(index.symbols.iter().all(|s| s.help.is_none()));
}

#[test]
fn unsupported_metadata_encoding_is_a_whole_file_failure() {
    let (_temp, pkg) = package_copy();
    std::fs::copy(
        fixture("migration/altrep-title.rds"),
        pkg.join("Meta/Rd.rds"),
    )
    .unwrap();
    let index = harvest_package(&pkg, HarvestOptions::default(), 0).unwrap();
    assert!(index.symbols.iter().all(|s| s.help.is_none()));
    assert!(index.symbols.iter().any(|s| s.formals.is_some()));
}

fn corrupt_record(pkg: &Path, directory: &str, name: &str) {
    let stem = pkg.join(directory).join("readerfixture");
    let index = rd_rds::lazyload::LazyLoadIndex::open(stem.with_extension("rdx")).unwrap();
    let location = index.variable(name).unwrap().location().unwrap();
    let path = stem.with_extension("rdb");
    let mut bytes = std::fs::read(&path).unwrap();
    let start = location.offset() as usize;
    bytes[start..start + location.length() as usize].fill(0);
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn corrupt_help_topic_keeps_title_and_other_topic_body() {
    let (_temp, pkg) = package_copy();
    corrupt_record(&pkg, "help", "zero");
    let index = harvest_package(&pkg, HarvestOptions::default(), 0).unwrap();
    let zero = index
        .symbols
        .iter()
        .find(|s| s.name == "zero")
        .unwrap()
        .help
        .as_ref()
        .unwrap();
    assert_eq!(zero.title.as_deref(), Some("Metadata title"));
    assert!(zero.description.is_none());
    assert!(zero.usage.is_none());
    assert!(zero.arguments.is_empty());
    let other = index
        .symbols
        .iter()
        .find(|s| s.name == "number")
        .unwrap()
        .help
        .as_ref()
        .unwrap();
    assert_eq!(other.usage.as_deref(), Some("zero()"));
}

#[test]
fn corrupt_code_record_keeps_other_signatures_and_metadata() {
    let (_temp, pkg) = package_copy();
    corrupt_record(&pkg, "R", "defaults");
    let index = harvest_package(&pkg, HarvestOptions::default(), 0).unwrap();
    let bad = index.symbols.iter().find(|s| s.name == "defaults").unwrap();
    assert!(bad.formals.is_none());
    assert!(bad.help.as_ref().unwrap().description.is_some());
    assert_eq!(
        index
            .symbols
            .iter()
            .find(|s| s.name == "zero")
            .unwrap()
            .formals
            .as_ref()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn renamed_directory_with_dotted_package_name_uses_description_stem() {
    let (_temp, pkg) = package_copy();
    let description = std::fs::read_to_string(pkg.join("DESCRIPTION"))
        .unwrap()
        .replace("Package: readerfixture", "Package: reader.fixture");
    std::fs::write(pkg.join("DESCRIPTION"), description).unwrap();
    for directory in ["R", "help"] {
        for extension in ["rdx", "rdb"] {
            std::fs::rename(
                pkg.join(directory)
                    .join(format!("readerfixture.{extension}")),
                pkg.join(directory)
                    .join(format!("reader.fixture.{extension}")),
            )
            .unwrap();
        }
    }
    let index = harvest_package(&pkg, HarvestOptions::default(), 0).unwrap();
    let zero = index.symbols.iter().find(|s| s.name == "zero").unwrap();
    assert_eq!(zero.formals.as_ref().unwrap().len(), 0);
    assert_eq!(zero.help.as_ref().unwrap().usage.as_deref(), Some("zero()"));
}

#[test]
fn duplicate_metadata_alias_uses_first_row_even_without_help_files() {
    let (_temp, pkg) = package_copy();
    std::fs::write(pkg.join("NAMESPACE"), "export(shared)\n").unwrap();
    std::fs::remove_dir_all(pkg.join("help")).unwrap();
    let index = harvest_package(&pkg, HarvestOptions::default(), 0).unwrap();
    let shared = index
        .symbols
        .iter()
        .find(|s| s.name == "shared")
        .unwrap()
        .help
        .as_ref()
        .unwrap();
    assert_eq!(shared.title.as_deref(), Some("Metadata title"));
    assert!(shared.description.is_none());
}

#[test]
fn empty_help_metadata_and_disabled_help_preserve_code_inspection() {
    let (_temp, pkg) = package_copy();
    std::fs::copy(fixture("migration/empty.rds"), pkg.join("Meta/Rd.rds")).unwrap();
    for help in [true, false] {
        let index = harvest_package(&pkg, HarvestOptions { help }, 0).unwrap();
        assert!(index.symbols.iter().all(|s| s.help.is_none()));
        assert!(index.symbols.iter().any(|s| s.formals.is_some()));
    }
}

#[test]
fn changed_data_file_invalidates_open_handle() {
    let (_temp, pkg) = package_copy();
    let path = pkg.join("R/readerfixture.rdx");
    let db = LazyLoadDb::open(&path).unwrap();
    std::fs::write(path.with_extension("rdb"), b"changed").unwrap();
    assert!(db.fetch("zero").is_err());
    assert!(db.contains("zero"));
}

#[test]
fn opaque_help_field_preserves_other_fields_without_using_legacy_decoder() {
    let (_temp, pkg) = package_copy();
    for extension in ["rdx", "rdb"] {
        std::fs::copy(
            fixture(&format!("migration/opaque-help.{extension}")),
            pkg.join("help").join(format!("readerfixture.{extension}")),
        )
        .unwrap();
    }
    let index = harvest_package(&pkg, HarvestOptions::default(), 0).unwrap();
    let zero = index
        .symbols
        .iter()
        .find(|s| s.name == "zero")
        .unwrap()
        .help
        .as_ref()
        .unwrap();
    assert_eq!(zero.title.as_deref(), Some("Metadata title"));
    assert!(zero.description.is_none());
    assert_eq!(zero.usage.as_deref(), Some("zero()"));
    assert_eq!(zero.arguments[0].name, "x, y");
}

#[test]
fn invalid_help_encoding_keeps_metadata_and_other_topics() {
    let (_temp, pkg) = package_copy();
    for extension in ["rdx", "rdb"] {
        std::fs::copy(
            fixture(&format!("migration/invalid-encoding-help.{extension}")),
            pkg.join("help").join(format!("readerfixture.{extension}")),
        )
        .unwrap();
    }
    let index = harvest_package(&pkg, HarvestOptions::default(), 0).unwrap();
    let zero = index
        .symbols
        .iter()
        .find(|s| s.name == "zero")
        .unwrap()
        .help
        .as_ref()
        .unwrap();
    assert_eq!(zero.title.as_deref(), Some("Metadata title"));
    assert!(zero.description.is_none());
    assert!(zero.usage.is_none());
    let other = index
        .symbols
        .iter()
        .find(|s| s.name == "number")
        .unwrap()
        .help
        .as_ref()
        .unwrap();
    assert!(other.description.is_some());
}

#[test]
fn invalid_title_encoding_recovers_the_page_title() {
    let (_temp, pkg) = package_copy();
    std::fs::copy(
        fixture("migration/invalid-encoding-title.rds"),
        pkg.join("Meta/Rd.rds"),
    )
    .unwrap();
    let index = harvest_package(&pkg, HarvestOptions::default(), 0).unwrap();
    let zero = index
        .symbols
        .iter()
        .find(|s| s.name == "zero")
        .unwrap()
        .help
        .as_ref()
        .unwrap();
    assert_eq!(zero.title.as_deref(), Some("Page title"));
    assert!(zero.description.is_some());
}

#[test]
fn shared_metadata_matches_every_existing_fixture_alias_and_title() {
    use arity::rindex::rds;
    use rd_helpdb::HelpTopicIndex;

    for (package, row_count, alias_count) in
        [("magrittr", 15, 49), ("R.oo", 182, 683), ("lazydata", 2, 2)]
    {
        let path = fixture(package);
        let stored = rds::read_rds(&std::fs::read(path.join("Meta/Rd.rds")).unwrap()).unwrap();
        let labels = stored.names().unwrap();
        let column = |name| {
            &stored.as_list().unwrap()[labels
                .iter()
                .position(|label| *label == Some(name))
                .unwrap()]
        };
        let aliases = column("Aliases").as_list().unwrap();
        let titles = column("Title").as_str_vec().unwrap();
        let files = column("File").as_str_vec().unwrap();
        let shared = HelpTopicIndex::read_installed(&path).unwrap().unwrap();
        assert_eq!(shared.len(), row_count);
        let mut first_rows = std::collections::BTreeMap::new();
        for (row, entry) in shared.entries().enumerate() {
            assert_eq!(&entry.aliases, aliases[row].as_str_vec().unwrap());
            assert_eq!(entry.title.as_str(), titles[row].as_deref());
            assert_eq!(entry.file.as_str(), files[row].as_deref());
            for alias in aliases[row].as_str_vec().unwrap().iter().flatten() {
                first_rows.entry(alias).or_insert(row);
            }
        }
        assert_eq!(first_rows.len(), alias_count);
        for (alias, row) in first_rows {
            let entry = shared.find_alias(alias).unwrap();
            assert_eq!(entry.title.as_str(), titles[row].as_deref());
            assert_eq!(
                entry.topic_key(),
                files[row]
                    .as_deref()
                    .map(|file| file.strip_suffix(".Rd").unwrap_or(file))
            );
        }
    }
}

#[test]
fn failed_attach_vector_does_not_discard_code_or_help() {
    use arity::rindex::{harvest::harvest_package_in, libpaths::LibrarySearch};

    let (_temp, pkg) = package_copy();
    let search = LibrarySearch::from_dirs(vec![fixture("")]);
    let before = harvest_package_in(&pkg, HarvestOptions::default(), 0, &search).unwrap();
    assert_eq!(
        before
            .attaches
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>(),
        ["R.oo", "magrittr"]
    );
    corrupt_record(&pkg, "R", "core");
    let after = harvest_package_in(&pkg, HarvestOptions::default(), 0, &search).unwrap();
    assert!(after.attaches.is_empty());
    let zero = after.symbols.iter().find(|s| s.name == "zero").unwrap();
    assert!(zero.formals.is_some());
    assert!(zero.help.as_ref().unwrap().usage.is_some());
}
