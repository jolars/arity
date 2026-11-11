//! Runtime facts are pinned for ordinary Rust tests; only the ignored test runs R.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use arity::formatter::format;
use arity::parser::{Edit, parse, reparse};
use arity::project::classes::{ClassName, ClassSystem, GeneratorRef, ParentRef, file_class_defs};
use arity::semantic::SemanticModel;
use serde::{Deserialize, Serialize};

fn fixture_names() -> &'static [&'static str] {
    &["s3", "s4", "reference_classes", "r6", "s7"]
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/oop_oracle")
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Versions {
    r: String,
    r6: String,
    s7: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Pin {
    versions: Versions,
    // Bind observations to their exact input so a source edit cannot leave stale facts.
    source: String,
    observer: String,
    facts: Facts,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Facts {
    system: String,
    classes: Vec<Class>,
    constructors: Vec<Constructor>,
    methods: Vec<Method>,
    dispatch: Vec<Dispatch>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Class {
    name: String,
    binding: String,
    parents: Vec<String>,
    members: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Formal {
    name: String,
    default: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Constructor {
    binding: String,
    formals: Vec<Formal>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Method {
    generic: String,
    signature: Vec<String>,
    formals: Vec<Formal>,
    // Frames from the method enclosure up to, but excluding, the fixture's lexical owner.
    environment: Vec<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Dispatch {
    expression: String,
    classes: Vec<String>,
    value: String,
}

fn pin(name: &str) -> Pin {
    serde_json::from_str(
        &fs::read_to_string(root().join(name).join("expected.json")).expect("read OOP pin"),
    )
    .expect("decode OOP runtime facts")
}

#[test]
fn pinned_facts_are_registered_and_current() {
    let versions: Versions =
        serde_json::from_str(include_str!("fixtures/oop_oracle/versions.json")).unwrap();
    let mut directories: Vec<_> = fs::read_dir(root())
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| entry.file_type().unwrap().is_dir())
        .map(|entry| entry.file_name().into_string().unwrap())
        .collect();
    directories.sort();
    let mut registered = fixture_names().to_vec();
    registered.sort();
    assert_eq!(directories, registered);
    for name in fixture_names() {
        let pin = pin(name);
        assert_eq!(pin.versions, versions, "{name}");
        assert_eq!(pin.facts.system, *name);
        assert_eq!(
            pin.source,
            fs::read_to_string(root().join(name).join("input.R")).unwrap(),
            "{name}: refresh and review the runtime facts after changing input"
        );
        assert_eq!(
            pin.observer,
            fs::read_to_string(root().join(name).join("observe.R")).unwrap(),
            "{name}: refresh and review the facts after changing observations"
        );
        assert!(!pin.facts.classes.is_empty(), "{name}");
        assert!(!pin.facts.constructors.is_empty(), "{name}");
        assert!(!pin.facts.methods.is_empty(), "{name}");
        assert!(!pin.facts.dispatch.is_empty(), "{name}");
        for class in &pin.facts.classes {
            assert!(!class.members.is_empty(), "{name}: {class:?}");
        }
    }
}

#[test]
fn supported_class_projection_matches_runtime() {
    for (name, system) in [
        ("s4", ClassSystem::S4),
        ("reference_classes", ClassSystem::RefClass),
        ("r6", ClassSystem::R6),
    ] {
        let pin = pin(name);
        let root = parse(&pin.source).cst;
        let classes = file_class_defs(&SemanticModel::build(&root), &root);
        assert_eq!(
            classes.declarations.len(),
            pin.facts.classes.len(),
            "{name}"
        );
        for class in &pin.facts.classes {
            let actual = classes
                .declarations
                .iter()
                .find(|def| def.name == ClassName::Known(class.name.clone()))
                .expect("runtime class");
            assert_eq!(actual.system(), system, "{name}: {}", class.name);
            assert_eq!(
                actual.binding.as_deref(),
                Some(class.binding.as_str()),
                "{name}: {}",
                class.name
            );
            let parents: Vec<_> = actual
                .parents
                .iter()
                .map(|parent| match parent {
                    ParentRef::Formal(name) => name.clone(),
                    ParentRef::Generator(GeneratorRef::Local(key)) => {
                        classes.declaration(key).unwrap().display_name().to_string()
                    }
                    other => panic!("unexpected parent {other:?}"),
                })
                .collect();
            assert_eq!(parents, class.parents, "{name}: {}", class.name);
        }
    }
}

#[test]
fn oracle_sources_preserve_parser_and_formatter_invariants() {
    let mut fallbacks = 0;
    for name in fixture_names() {
        let source = fs::read_to_string(root().join(name).join("input.R")).unwrap();
        let old = parse(&source);
        assert!(old.diagnostics.is_empty(), "{name}: {:?}", old.diagnostics);
        assert_eq!(old.cst.text().to_string(), source, "{name}");
        let formatted = format(&source).unwrap();
        assert_eq!(format(&formatted).unwrap(), formatted, "{name}");
        let formatted_parse = parse(&formatted);
        assert!(formatted_parse.diagnostics.is_empty(), "{name}");
        assert_eq!(formatted_parse.cst.text().to_string(), formatted, "{name}");

        let mut incremental = 0;
        for offset in source
            .char_indices()
            .map(|(offset, _)| offset)
            .chain(std::iter::once(source.len()))
        {
            for (delete, insert) in [(false, "z"), (false, "\n"), (false, "\""), (true, "")] {
                let deleted_bytes = if delete {
                    let Some(ch) = source[offset..].chars().next() else {
                        continue;
                    };
                    ch.len_utf8()
                } else {
                    0
                };
                let edit = Edit {
                    range: offset..offset + deleted_bytes,
                    insert: insert.into(),
                };
                let target = edit.apply(&source);
                let full = parse(&target);
                assert_eq!(full.cst.text().to_string(), target, "{name}: {edit:?}");
                let Some(mut reparsed) = reparse(&old.cst, &source, &old.diagnostics, &edit) else {
                    fallbacks += 1;
                    continue;
                };
                incremental += 1;
                assert_eq!(
                    reparsed.green,
                    full.cst.green().to_owned(),
                    "{name}: {edit:?}"
                );
                let mut diagnostics = full.diagnostics;
                let key = |d: &arity::parser::ParseDiagnostic| (d.start, d.end, d.message.clone());
                reparsed.diagnostics.sort_by_key(key);
                diagnostics.sort_by_key(key);
                assert_eq!(reparsed.diagnostics, diagnostics, "{name}: {edit:?}");
            }
        }
        assert!(incremental > 0, "{name}: must exercise incremental parsing");
    }
    assert!(fallbacks > 0, "must exercise full-parse fallback");
}

#[test]
#[ignore = "requires pinned R, R6, and S7; run via `task oop-oracle`"]
fn live_runtime_matches_pins_and_formatted_sources() {
    let refresh = std::env::var_os("UPDATE_OOP_ORACLE").is_some();
    let rscript = std::env::var_os("RSCRIPT").unwrap_or_else(|| "Rscript".into());
    let driver = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/oop_oracle.R");
    let versions: Versions =
        serde_json::from_str(include_str!("fixtures/oop_oracle/versions.json")).unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let mut updates = Vec::new();
    for name in fixture_names() {
        let fixture = root().join(name);
        let input = fs::read_to_string(fixture.join("input.R")).unwrap();
        let formatted_path = temporary.path().join(format!("{name}.R"));
        fs::write(&formatted_path, format(&input).unwrap()).unwrap();
        let run = |source: &Path| {
            let output = Command::new(&rscript)
                .arg("--vanilla")
                .arg(&driver)
                .arg(&fixture)
                .arg(source)
                .args([&versions.r, &versions.r6, &versions.s7])
                .output()
                .expect("run Rscript (see tests/fixtures/oop_oracle/README.md)");
            assert!(
                output.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            serde_json::from_slice::<serde_json::Value>(&output.stdout)
                .unwrap_or_else(|error| panic!("{name}: {error}: {output:?}"))
        };
        let actual = run(&fixture.join("input.R"));
        let formatted = run(&formatted_path);
        assert_eq!(
            actual, formatted,
            "{name}: formatting changed R's observations"
        );
        let pin = serde_json::json!({
            "versions": actual["versions"],
            "source": input,
            "observer": fs::read_to_string(fixture.join("observe.R")).unwrap(),
            "facts": actual["facts"]
        });
        // Validate the schema before writing any pins, including during refresh.
        serde_json::from_value::<Pin>(pin.clone()).expect("runtime fact schema");
        let destination = fixture.join("expected.json");
        if refresh {
            updates.push((
                destination,
                serde_json::to_string_pretty(&pin).unwrap() + "\n",
            ));
        } else {
            let expected: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(destination).unwrap()).unwrap();
            assert_eq!(
                pin, expected,
                "{name}: runtime facts differ; inspect before refreshing"
            );
        }
    }
    // A failed case must not leave a partially refreshed corpus.
    for (destination, contents) in updates {
        fs::write(destination, contents).unwrap();
    }
}

#[test]
#[ignore = "requires pinned R, R6, and S7; run via `task oop-oracle`"]
fn live_runtime_rejects_unpinned_versions() {
    let versions: Versions =
        serde_json::from_str(include_str!("fixtures/oop_oracle/versions.json")).unwrap();
    let rscript = std::env::var_os("RSCRIPT").unwrap_or_else(|| "Rscript".into());
    let driver = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/oop_oracle.R");
    for index in 0..3 {
        let mut requested = [&versions.r[..], &versions.r6[..], &versions.s7[..]];
        requested[index] = "unpinned";
        let output = Command::new(&rscript)
            .arg("--vanilla")
            .arg(&driver)
            .arg(root().join("r6"))
            .arg(root().join("r6/input.R"))
            .args(requested)
            .output()
            .expect("run Rscript");
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("OOP oracle requires R/R6/S7"));
        assert!(
            output.stdout.is_empty(),
            "must reject before evaluating fixtures"
        );
    }
}
