//! JSON Schema for `arity.toml`.
//!
//! The checked-in schema is generated from the configuration types. Set
//! `UPDATE_EXPECTED=1` to regenerate it after an intentional configuration
//! change, then review the resulting diff.

use std::fs;
use std::path::{Path, PathBuf};

use arity::config::Config;
use jsonschema::Validator;
use serde_json::Value;

const SCHEMA_ID: &str = "https://arity.cc/arity.schema.json";
const SCHEMA_PATH: &str = "arity.schema.json";

fn generate_schema_json() -> Value {
    let schema = schemars::schema_for!(Config);
    let mut json = serde_json::to_value(schema).expect("schema to JSON");
    let Value::Object(root) = &mut json else {
        panic!("a JSON Schema root must be an object");
    };

    root.insert("$id".into(), SCHEMA_ID.into());
    root.insert("title".into(), "Arity configuration".into());
    root.insert(
        "description".into(),
        "Schema for arity.toml. Generated from Arity's configuration types; do not hand-edit—run `UPDATE_EXPECTED=1 cargo test --test config_schema` instead."
            .into(),
    );
    json
}

fn schema_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(SCHEMA_PATH)
}

fn pretty_json(json: &Value) -> String {
    let mut output = serde_json::to_string_pretty(json).expect("serialize schema");
    output.push('\n');
    output
}

fn validator() -> Validator {
    Validator::new(&generate_schema_json()).expect("compile schema")
}

fn toml_to_json(toml: &str) -> Value {
    let value: toml::Value = toml::from_str(toml).expect("parse TOML");
    serde_json::to_value(value).expect("TOML to JSON")
}

fn assert_rejected(toml: &str) {
    let json = toml_to_json(toml);
    let errors: Vec<_> = validator()
        .iter_errors(&json)
        .map(|error| error.to_string())
        .collect();
    assert!(!errors.is_empty(), "schema unexpectedly accepted:\n{toml}");
}

#[test]
fn schema_is_in_sync_with_config_types() {
    let generated = pretty_json(&generate_schema_json());
    let path = schema_path();

    if std::env::var_os("UPDATE_EXPECTED").is_some() {
        fs::write(&path, generated).expect("write schema");
        return;
    }

    let checked_in = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing {}: {error}. Run `UPDATE_EXPECTED=1 cargo test --test config_schema` to create it.",
            path.display()
        )
    });
    similar_asserts::assert_eq!(
        checked_in,
        generated,
        "{} is out of date; regenerate it with `UPDATE_EXPECTED=1 cargo test --test config_schema`",
        path.display()
    );
}

#[test]
fn schema_accepts_the_repository_config() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("arity.toml");
    let toml = fs::read_to_string(&path).expect("read repository config");
    Config::load_from(&path).expect("repository config is accepted by Arity");

    let json = toml_to_json(&toml);
    let errors: Vec<_> = validator()
        .iter_errors(&json)
        .map(|error| error.to_string())
        .collect();
    assert!(
        errors.is_empty(),
        "schema rejected {}:\n{}",
        path.display(),
        errors.join("\n")
    );
}

#[test]
fn schema_accepts_every_public_setting() {
    let toml = r#"
exclude = ["vendor/"]
extend-exclude = ["generated/"]
cache = false

[format]
line-width = 100
indent-width = 4
line-ending = "crlf"
description = false

[lint]
select = ["undesirable-function"]
ignore = ["unused-binding"]

[lint.rules.undesirable-function]
functions = { attach = "avoid changing the search path" }
extend-functions = { sapply = "use `vapply()`" }

[compat]
r = "4.1"
roxygen2 = "7.3.2"

[index]
library-paths = ["/opt/R/library"]
cache-dir = ".arity-cache"
auto-build = false
help = false
"#;
    toml::from_str::<Config>(toml).expect("config is accepted by Arity");

    let json = toml_to_json(toml);
    let errors: Vec<_> = validator()
        .iter_errors(&json)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "schema errors:\n{}", errors.join("\n"));
}

#[test]
fn schema_rejects_unknown_keys() {
    assert_rejected("line-widht = 80");
    assert_rejected("[format]\nline-widht = 80");
    assert_rejected("[lint.rules]\nundesirabl-function = {}");
}

#[test]
fn schema_rejects_invalid_values() {
    assert_rejected("[format]\nline-ending = \"windows\"");
    assert_rejected("[format]\nline-width = 0");
    assert_rejected("[format]\nindent-width = 1001");
    assert_rejected("[compat]\nr = \"version four\"");
}

#[test]
fn schema_omits_machine_only_and_resolved_settings() {
    assert_rejected("[index]\nremote-url = \"https://example.com\"");
    assert_rejected("[lint.compat]\nr = \"4.1\"");
}
