//! Integration tests for the R-introspection harvester, run against real
//! installed-package bytes checked in under `tests/fixtures/rindex/`. These
//! require **no R** — the bytes are the ground truth.

use std::path::PathBuf;

use arity::rindex::harvest::{HarvestOptions, harvest_package, harvest_package_in};
use arity::rindex::lazyload::LazyLoadDb;
use arity::rindex::libpaths::LibrarySearch;
use arity::rindex::rds::{self, Rkind};
use arity::rindex::schema::{
    Formal, InstalledMemberKind, ObjectSystem, ObjectType, SymbolEntry, SymbolKind,
};

fn fixture(pkg: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rindex")
        .join(pkg)
}

/// The checked-in fixture tree as a (hermetic) library search path.
fn fixture_lib() -> LibrarySearch {
    LibrarySearch::from_dirs(vec![
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rindex"),
    ])
}

fn find<'a>(symbols: &'a [SymbolEntry], name: &str) -> Option<&'a SymbolEntry> {
    symbols.iter().find(|s| s.name == name)
}

#[test]
fn reads_lazyload_object_names() {
    let rdx = fixture("magrittr").join("R/magrittr.rdx");
    let db = LazyLoadDb::open(&rdx).expect("open magrittr lazy-load db");
    let names: Vec<&str> = db.names().collect();
    assert!(names.contains(&"%>%"), "expected pipe operator in objects");
    assert!(names.contains(&"freduce"));
    // The latin1/utf8 alias must round-trip.
    assert!(names.iter().any(|n| n.contains("est pas")));
}

#[test]
fn reads_rd_index_data_frame() {
    let bytes = std::fs::read(fixture("magrittr").join("Meta/Rd.rds")).unwrap();
    let rd = rds::read_rds(&bytes).expect("parse Rd.rds");
    let cols = rd.names().expect("data.frame columns");
    assert!(cols.contains(&Some("Name")));
    assert!(cols.contains(&Some("Title")));
    assert!(cols.contains(&Some("Aliases")));
}

#[test]
fn harvests_the_packages_own_title() {
    // The package-level `Title:`, distinct from a *symbol's* Rd help title. It
    // is what a DESCRIPTION dependency hovers to, and what labels an installed
    // package in dependency-field completion — where reading each candidate's
    // DESCRIPTION off disk is not an option.
    let idx = harvest_package(&fixture("magrittr"), HarvestOptions::default(), 0)
        .expect("harvest magrittr");
    assert_eq!(
        idx.title.as_deref(),
        Some("A Forward-Pipe Operator for R"),
        "the DESCRIPTION's own Title field"
    );
}

#[test]
fn harvests_explicit_exports_with_titles() {
    let idx = harvest_package(&fixture("magrittr"), HarvestOptions::default(), 0)
        .expect("harvest magrittr");
    assert_eq!(idx.package, "magrittr");
    assert_eq!(idx.version, "2.0.4");

    // Explicit exports, including quoted operators.
    assert!(find(&idx.symbols, "%>%").is_some());
    assert!(find(&idx.symbols, "freduce").is_some());
    assert!(find(&idx.symbols, "n'est pas").is_some());

    // Help title resolved via the alias → title map.
    let pipe = find(&idx.symbols, "%<>%").expect("%<>% exported");
    let title = pipe.help.as_ref().and_then(|h| h.title.as_deref());
    assert_eq!(title, Some("Assignment pipe"));

    // Symbols are sorted.
    let mut sorted = idx.symbols.clone();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(idx.symbols, sorted);
}

#[test]
fn harvests_attach_set_from_core_variable() {
    // metatoy follows the tidyverse convention: a `core` character vector in
    // the namespace names what `.onAttach` attaches. Both members are fixture
    // packages, so the installed-member validation passes.
    let idx = harvest_package_in(
        &fixture("metatoy"),
        HarvestOptions::default(),
        0,
        &fixture_lib(),
    )
    .expect("harvest metatoy");
    assert_eq!(idx.package, "metatoy");
    let attaches: Vec<&str> = idx.attaches.iter().map(|s| s.as_str()).collect();
    assert_eq!(attaches, ["R.oo", "magrittr"]);
    // The ordinary harvest is unaffected.
    assert!(find(&idx.symbols, "metatoy_hello").is_some());
}

#[test]
fn ordinary_packages_harvest_empty_attach_sets() {
    for pkg in ["magrittr", "R.oo"] {
        let idx = harvest_package_in(&fixture(pkg), HarvestOptions::default(), 0, &fixture_lib())
            .expect("harvest");
        assert!(
            idx.attaches.is_empty(),
            "{pkg} should not capture an attach set"
        );
    }
}

#[test]
fn attach_capture_requires_installed_members() {
    // The plain `harvest_package` wrapper has no library search path, so the
    // members cannot be validated as installed and nothing is recorded — a
    // partial or unverifiable set must never be preferred over the static
    // fallback table.
    let idx = harvest_package(&fixture("metatoy"), HarvestOptions::default(), 0)
        .expect("harvest metatoy");
    assert!(idx.attaches.is_empty());
}

/// End-to-end probe test: needs `R` on `PATH` (devenv provides it), so it is
/// `#[ignore]`d like the roxygen oracles. Run with
/// `cargo test --test rindex -- --ignored attach_probe`.
#[test]
#[ignore = "requires R on PATH"]
fn attach_probe_captures_search_diff() {
    use arity::rindex::build::{BuildOptions, build_index};
    use arity::rindex::cache::Cache;

    // A toy package whose `.onAttach` exists but that follows no attach-set
    // convention — only the probe can see what `library()` attaches (here its
    // `Depends: tools`, attached by R itself; `tools` is installed wherever R
    // is but not attached by default, so the diff is exactly `tools`).
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("probetoy");
    std::fs::create_dir_all(src.join("R")).unwrap();
    std::fs::write(
        src.join("DESCRIPTION"),
        "Package: probetoy\nVersion: 1.0.0\nTitle: Probe Fixture\n\
         Description: Throwaway fixture for the attach probe.\n\
         License: MIT + file LICENSE\nDepends: tools\n",
    )
    .unwrap();
    std::fs::write(src.join("NAMESPACE"), "export(probetoy_hello)\n").unwrap();
    std::fs::write(
        src.join("R/probetoy.R"),
        ".onAttach <- function(libname, pkgname) {\n\
         \x20 packageStartupMessage(\"probetoy attached\")\n\
         }\n\
         probetoy_hello <- function() \"hello\"\n",
    )
    .unwrap();

    let lib = tmp.path().join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    let status = std::process::Command::new("R")
        .args(["CMD", "INSTALL", "--no-docs", "-l"])
        .arg(&lib)
        .arg(&src)
        .status()
        .expect("R must be on PATH for this test");
    assert!(status.success(), "R CMD INSTALL failed");

    let cache = Cache::new(tmp.path().join("cache"));
    let search = LibrarySearch::discover(None, &[lib]);
    let pkgs = [smol_str::SmolStr::new("probetoy")];

    // Probe off (the default): the heuristic finds nothing.
    build_index(&pkgs, &cache, &search, BuildOptions::default(), 0);
    let idx = cache.read_package("probetoy", "1.0.0").expect("indexed");
    assert!(idx.attaches.is_empty(), "heuristic should find nothing");

    // Probe on, no force: the up-to-date entry is probed and merged in place.
    build_index(
        &pkgs,
        &cache,
        &search,
        BuildOptions {
            attach_probe: true,
            ..Default::default()
        },
        0,
    );
    let idx = cache.read_package("probetoy", "1.0.0").expect("indexed");
    let attaches: Vec<&str> = idx.attaches.iter().map(|s| s.as_str()).collect();
    assert_eq!(attaches, ["tools"]);
}

#[test]
fn harvests_export_pattern_package() {
    let idx =
        harvest_package(&fixture("R.oo"), HarvestOptions::default(), 0).expect("harvest R.oo");
    assert_eq!(idx.package, "R.oo");

    // `exportPattern("^[^\\.]")` exports non-dotted objects.
    assert!(find(&idx.symbols, "Object").is_some());
    assert!(find(&idx.symbols, "throw").is_some());
    assert!(find(&idx.symbols, "Exception").is_some());

    // Dotted internals are excluded by the pattern.
    assert!(find(&idx.symbols, ".onLoad").is_none());
    assert!(find(&idx.symbols, ".__NAMESPACE__.").is_none());
}

#[test]
fn fetches_compiled_closure_formals() {
    // `freduce` is a byte-compiled magrittr closure: fetching it requires
    // consuming a BCODESXP body, and its formals must come back intact.
    let rdx = fixture("magrittr").join("R/magrittr.rdx");
    let db = LazyLoadDb::open(&rdx).expect("open magrittr lazy-load db");
    let obj = db.fetch("freduce").expect("fetch freduce");
    let Rkind::Closure { formals, .. } = &obj.kind else {
        panic!("freduce should be a closure, got {:?}", obj.kind);
    };
    let names: Vec<&str> = match &formals.kind {
        Rkind::Pairlist(items) => items.iter().filter_map(|i| i.tag.as_deref()).collect(),
        other => panic!("formals should be a pairlist, got {other:?}"),
    };
    assert_eq!(names, ["value", "function_list"]);

    // A formal carrying `...` round-trips as a named, default-less argument.
    let dots = db.fetch("[.fseq").expect("fetch [.fseq");
    let Rkind::Closure { formals, .. } = &dots.kind else {
        panic!("[.fseq should be a closure");
    };
    let names: Vec<&str> = match &formals.kind {
        Rkind::Pairlist(items) => items.iter().filter_map(|i| i.tag.as_deref()).collect(),
        other => panic!("formals pairlist, got {other:?}"),
    };
    assert!(
        names.contains(&"..."),
        "expected a `...` formal, got {names:?}"
    );
}

#[test]
fn harvest_fills_function_formals() {
    let idx = harvest_package(&fixture("magrittr"), HarvestOptions::default(), 0)
        .expect("harvest magrittr");

    let freduce = find(&idx.symbols, "freduce").expect("freduce exported");
    assert_eq!(freduce.kind, SymbolKind::Function);
    let formals = freduce.formals.as_ref().expect("freduce has formals");
    let names: Vec<&str> = formals.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["value", "function_list"]);
    // Both are required → no default text.
    assert!(formals.iter().all(|f| f.default.is_none()));

    // A primitive aliased into the package stays a callable Function with no
    // R-level formals (not misclassified as data).
    let add = find(&idx.symbols, "add").expect("add exported");
    assert_eq!(add.kind, SymbolKind::Function);
    assert!(add.formals.is_none());
}

#[test]
fn harvest_tolerates_missing_rdb() {
    // R.oo ships only a `.rdx` (no `.rdb`): harvesting must still succeed, just
    // without formals.
    let idx =
        harvest_package(&fixture("R.oo"), HarvestOptions::default(), 0).expect("harvest R.oo");
    assert!(idx.symbols.iter().all(|s| s.formals.is_none()));
}

#[test]
fn help_can_be_disabled() {
    let idx = harvest_package(&fixture("magrittr"), HarvestOptions { help: false }, 0)
        .expect("harvest magrittr");
    assert!(idx.symbols.iter().all(|s| s.help.is_none()));
}

#[test]
fn harvests_full_help_body_from_help_db() {
    // magrittr ships `help/magrittr.{rdb,rdx}`: the pipe page resolves a full
    // body (title + description + usage + arguments), not just a title.
    let idx = harvest_package(&fixture("magrittr"), HarvestOptions::default(), 0)
        .expect("harvest magrittr");
    let pipe = find(&idx.symbols, "%>%").expect("%>% exported");
    let help = pipe.help.as_ref().expect("%>% has help");

    assert_eq!(help.title.as_deref(), Some("Pipe"));
    assert!(
        help.description
            .as_deref()
            .is_some_and(|d| d.to_lowercase().contains("pipe an object")),
        "description: {:?}",
        help.description
    );
    assert!(help.usage.as_deref().is_some_and(|u| u.contains("%>%")));
    assert!(
        help.arguments.iter().any(|a| a.name == "lhs"),
        "arguments: {:?}",
        help.arguments
    );

    // `\code{lhs}` inside the assignment-pipe description renders as a backtick
    // span, proving inline-macro markdown rendering (not just title passthrough).
    let apipe = find(&idx.symbols, "%<>%").expect("%<>% exported");
    let desc = apipe
        .help
        .as_ref()
        .and_then(|h| h.description.as_deref())
        .unwrap_or_default();
    assert!(
        desc.contains("`lhs`"),
        "expected backtick span, got: {desc}"
    );
}

#[test]
fn missing_help_db_degrades_to_title_only() {
    // R.oo ships no `help/` lazy-load DB: titles still resolve from Meta/Rd.rds,
    // but no symbol gets a description/usage/arguments body.
    let idx =
        harvest_package(&fixture("R.oo"), HarvestOptions::default(), 0).expect("harvest R.oo");
    assert!(
        idx.symbols
            .iter()
            .filter_map(|s| s.help.as_ref())
            .any(|h| h.title.is_some()),
        "expected at least one title from Meta/Rd.rds"
    );
    assert!(
        idx.symbols
            .iter()
            .filter_map(|s| s.help.as_ref())
            .all(|h| { h.description.is_none() && h.usage.is_none() && h.arguments.is_empty() }),
        "no Rd bodies should be present without a help DB"
    );
}

#[test]
fn harvests_lazydata_as_data_symbols() {
    // `lazydata` mimics `datasets`: its NAMESPACE exports nothing and it has no
    // `R/` code DB — every symbol comes from the `data/Rdata` lazy-load DB.
    let idx = harvest_package(&fixture("lazydata"), HarvestOptions::default(), 0)
        .expect("harvest lazydata");
    assert_eq!(idx.package, "lazydata");

    for name in ["toy_frame", "toy_vec"] {
        let sym = find(&idx.symbols, name).unwrap_or_else(|| panic!("{name} harvested"));
        assert_eq!(sym.kind, SymbolKind::Data, "{name} is data");
        assert!(sym.exported, "{name} available as pkg::{name}");
        assert!(sym.formals.is_none(), "{name} has no formals");
    }

    // Symbols stay sorted.
    let mut sorted = idx.symbols.clone();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(idx.symbols, sorted);
}

#[test]
fn lazydata_resolves_help_title() {
    // Lazy-data aliases resolve titles through the same `Meta/Rd.rds` path as
    // code exports.
    let idx = harvest_package(&fixture("lazydata"), HarvestOptions::default(), 0)
        .expect("harvest lazydata");
    let frame = find(&idx.symbols, "toy_frame").expect("toy_frame harvested");
    let title = frame.help.as_ref().and_then(|h| h.title.as_deref());
    assert_eq!(title, Some("A toy data frame"));
}

#[test]
fn help_body_snapshot() {
    let idx = harvest_package(&fixture("magrittr"), HarvestOptions::default(), 0)
        .expect("harvest magrittr");
    let pipe = find(&idx.symbols, "%>%").expect("%>% exported");
    insta::assert_debug_snapshot!(pipe.help);
}

#[test]
fn harvests_installed_classes_and_members() {
    let index = harvest_package(&fixture("oopfixture"), HarvestOptions::default(), 0).unwrap();
    let class = |name: &str| {
        index
            .objects
            .classes
            .iter()
            .find(|class| class.identity.name == name)
            .unwrap()
    };

    let shape = class("Shape");
    assert_eq!(shape.system, ObjectSystem::S4);
    assert_eq!(shape.origin.binding, ".__C__Shape");
    assert_eq!(shape.help.as_ref().unwrap().alias, "Shape-class");
    let slot = &shape.members[0];
    assert_eq!(slot.name, "size");
    assert_eq!(slot.kind, InstalledMemberKind::Slot);
    assert!(matches!(&slot.declared_type, Some(ObjectType::Named(ty)) if ty.name == "numeric"));

    let counter = class("Counter");
    assert_eq!(counter.system, ObjectSystem::R6);
    assert_eq!(counter.identity.package.as_deref(), Some("oopfixture"));
    assert_eq!(counter.parent_generator.as_ref().unwrap().name, "LocalBase");
    assert_eq!(class("BaseCounter").origin.binding, "LocalBase");
    assert!(find(&index.symbols, "LocalBase").is_none());
    assert_eq!(
        counter.help.as_ref().unwrap().topic.as_deref(),
        Some("Shape")
    );
    assert_eq!(
        counter.constructor.as_deref(),
        Some(
            [Formal {
                name: "start".into(),
                default: Some("0".into())
            }]
            .as_slice()
        )
    );
    assert!(
        counter
            .members
            .iter()
            .any(|member| member.name == "secret" && member.private)
    );
    assert!(counter.members.iter().any(|member| {
        member.name == "value" && member.kind == InstalledMemberKind::ActiveBinding
    }));

    let reference = class("RefCounter");
    assert_eq!(reference.system, ObjectSystem::Reference);
    assert_eq!(reference.parents[0].name, "envRefClass");
    assert_eq!(reference.generators[0].binding, "RefCounter");
    assert_eq!(reference.constructor.as_ref().unwrap()[0].name, "...");
    assert!(reference.members.iter().any(|member| {
        member.name == "add"
            && member.kind == InstalledMemberKind::Method
            && member.formals.as_ref().unwrap()[0].default.as_deref() == Some("1")
    }));

    let point = class("Point");
    assert_eq!(point.system, ObjectSystem::S7);
    assert_eq!(point.identity.package.as_deref(), Some("oopfixture"));
    assert_eq!(point.constructor.as_ref().unwrap()[0].name, "x");
    let property = &point.members[0];
    assert_eq!(property.kind, InstalledMemberKind::Property);
    assert!(matches!(&property.declared_type, Some(ObjectType::Named(ty)) if ty.name == "double"));
    assert_eq!(index.objects.classes.len(), 5);
    assert!(find(&index.symbols, "malformed").is_some());
    assert!(find(&index.symbols, "future").is_some());
}

#[test]
fn harvests_installed_generics_and_method_registrations() {
    let index = harvest_package(&fixture("oopfixture"), HarvestOptions::default(), 0).unwrap();
    for (system, name, args) in [
        (ObjectSystem::S3, "describe", None),
        (
            ObjectSystem::S4,
            "area",
            Some(vec!["x".into(), "scale".into()]),
        ),
        (ObjectSystem::S7, "measure", Some(vec!["x".into()])),
    ] {
        let generic = index
            .objects
            .generics
            .iter()
            .find(|generic| generic.system == system)
            .unwrap();
        assert_eq!(generic.identity.name, name);
        assert_eq!(generic.origin.binding, name);
        assert_eq!(generic.dispatch_arguments, args);
        assert_eq!(generic.formals.as_ref().unwrap()[0].name, "x");
    }

    let methods = &index.objects.methods;
    let s4 = methods
        .iter()
        .find(|method| method.system == ObjectSystem::S4)
        .unwrap();
    assert_eq!(s4.generic.name, "area");
    assert_eq!(s4.generic.package.as_deref(), Some("oopfixture"));
    assert_eq!(s4.origin.binding, ".__T__area:oopfixture");
    let dispatch = &s4.dispatch.as_ref().unwrap()[0];
    assert_eq!(dispatch.argument.as_deref(), Some("x"));
    assert!(matches!(&dispatch.class, Some(ObjectType::Named(ty))
        if ty.name == "Shape" && ty.package.as_deref() == Some("oopfixture")));
    assert_eq!(s4.help.as_ref().unwrap().alias, "area,Shape-method");
    assert_eq!(s4.target_dispatch, s4.dispatch);

    let s7 = methods
        .iter()
        .find(|method| method.generic.name == "measure")
        .unwrap();
    assert_eq!(s7.system, ObjectSystem::S7);
    assert_eq!(s7.origin.entry.as_deref(), Some("oopfixture::Point"));
    assert_eq!(
        s7.formals.as_ref().unwrap()[1].default.as_deref(),
        Some("1")
    );
    let dispatch = &s7.dispatch.as_ref().unwrap()[0];
    assert_eq!(dispatch.argument.as_deref(), Some("x"));
    assert!(matches!(&dispatch.class, Some(ObjectType::Named(ty)) if ty.name == "Point"));

    let external = methods
        .iter()
        .find(|method| method.generic.name == "foreign_measure")
        .unwrap();
    assert_eq!(external.generic.package.as_deref(), Some("otherfixture"));
    assert!(external.conditional);
    assert!(matches!(
        &external.dispatch.as_ref().unwrap()[0].class,
        Some(ObjectType::Union(types)) if types.len() == 2
    ));

    let s3: Vec<_> = methods
        .iter()
        .filter(|method| method.system == ObjectSystem::S3)
        .collect();
    assert_eq!(s3.len(), 2);
    assert_eq!(s3[0].generic.name, "describe");
    assert_eq!(s3[0].generic.package, None);
    assert_eq!(s3[1].generic.name, "print");
    assert_eq!(s3[1].generic.package.as_deref(), Some("base"));
    for method in s3 {
        assert_eq!(method.origin.binding, "describe_impl");
        let dispatch = &method.dispatch.as_ref().unwrap()[0];
        assert!(matches!(&dispatch.class, Some(ObjectType::Named(ty)) if ty.name == "fixture"));
        assert_eq!(
            method.formals.as_ref().unwrap()[2].default.as_deref(),
            Some("\"fixture\"")
        );
    }
    assert!(find(&index.symbols, "describe_impl").is_none());

    let temp = tempfile::tempdir().unwrap();
    let cache = arity::rindex::cache::Cache::new(temp.path().into());
    cache.write_package(&index).unwrap();
    assert_eq!(
        cache.read_package(&index.package, &index.version),
        Some(index.clone())
    );
    let names = cache
        .read_package_exports(&index.package, &index.version)
        .unwrap();
    assert_eq!(
        names
            .symbols
            .iter()
            .map(|symbol| &symbol.name)
            .collect::<Vec<_>>(),
        index
            .symbols
            .iter()
            .map(|symbol| &symbol.name)
            .collect::<Vec<_>>()
    );
}

fn copy_oopfixture_index(destination: &std::path::Path) {
    let source = fixture("oopfixture");
    std::fs::create_dir_all(destination.join("R")).unwrap();
    for file in ["DESCRIPTION", "NAMESPACE", "R/oopfixture.rdx"] {
        std::fs::copy(source.join(file), destination.join(file)).unwrap();
    }
}

#[test]
fn retains_names_and_s3_tuples_when_code_records_are_missing() {
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path();
    let source = fixture("oopfixture");
    copy_oopfixture_index(package);
    let options = HarvestOptions { help: false };
    let complete = harvest_package(&source, options, 0).unwrap();
    assert!(
        complete
            .objects
            .classes
            .iter()
            .all(|class| class.help.is_none())
    );
    let partial = harvest_package(package, options, 0).unwrap();
    assert_eq!(
        complete
            .symbols
            .iter()
            .map(|symbol| &symbol.name)
            .collect::<Vec<_>>(),
        partial
            .symbols
            .iter()
            .map(|symbol| &symbol.name)
            .collect::<Vec<_>>()
    );
    assert!(partial.objects.classes.is_empty());
    assert!(partial.objects.generics.is_empty());
    assert_eq!(partial.objects.methods.len(), 2);
    assert!(partial.objects.methods.iter().all(|method| {
        method.system == ObjectSystem::S3 && method.formals.is_none() && method.dispatch.is_some()
    }));
}

#[test]
fn skips_a_corrupt_lazy_load_reference_and_keeps_other_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let source = fixture("oopfixture");
    copy_oopfixture_index(temp.path());
    let database = LazyLoadDb::open(&source.join("R/oopfixture.rdx")).unwrap();
    let Rkind::Persistent(keys) = database.fetch("Counter").unwrap().kind else {
        panic!("expected a persisted generator");
    };
    let key = keys[0].as_deref().unwrap();
    let rdx = rd_rds::lazyload::LazyLoadIndex::open(source.join("R/oopfixture.rdx")).unwrap();
    let rd_rds::lazyload::RecordReference::Direct(location) = rdx.reference(key).unwrap() else {
        panic!("expected a direct generator record");
    };
    let mut rdb = std::fs::read(source.join("R/oopfixture.rdb")).unwrap();
    let offset = location.offset() as usize;
    rdb[offset..offset + location.length() as usize].fill(0);
    std::fs::write(temp.path().join("R/oopfixture.rdb"), rdb).unwrap();

    let index = harvest_package(temp.path(), HarvestOptions { help: false }, 0).unwrap();
    assert!(find(&index.symbols, "Counter").is_some());
    assert!(
        index
            .objects
            .classes
            .iter()
            .all(|class| class.identity.name != "Counter")
    );
    assert!(
        index
            .objects
            .classes
            .iter()
            .any(|class| class.identity.name == "Shape")
    );
}
