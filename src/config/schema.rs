//! Generation and parity tests for the published `arity.toml` JSON Schema.
//!
//! The schema is a generated artifact. Set `UPDATE_EXPECTED=1` and run
//! `cargo test config_schema` after an intentional configuration change, then
//! review the resulting `arity.schema.json` diff.

use std::fs;
use std::path::{Path, PathBuf};

use jsonschema::Validator;
use schemars::generate::SchemaSettings;
use schemars::transform::RestrictFormats;
use serde_json::Value;

use super::Config;

const SCHEMA_ID: &str = "https://arity.cc/arity.schema.json";
const SCHEMA_PATH: &str = "arity.schema.json";

fn generate_schema_json() -> Value {
    // SchemaStore recommends draft 7 and validates schemas in strict mode.
    // Rust-specific formats such as `uint32` add no constraint beyond the
    // integer bounds and are not portable across its supported validators.
    let generator = SchemaSettings::draft07()
        .with_transform(RestrictFormats::default())
        .into_generator();
    let schema = generator.into_root_schema_for::<Config>();
    let mut json = serde_json::to_value(schema).expect("serialize configuration schema");
    let Value::Object(root) = &mut json else {
        panic!("root configuration schema must be an object");
    };

    root.insert("$id".into(), SCHEMA_ID.into());
    root.insert("title".into(), "Arity configuration".into());
    root.insert(
        "description".into(),
        "Schema for arity.toml. Generated from Arity's configuration types; do not hand-edit—run `UPDATE_EXPECTED=1 cargo test config_schema` instead."
            .into(),
    );
    json
}

fn schema_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(SCHEMA_PATH)
}

fn render_schema(schema: &Value) -> String {
    let mut rendered = serde_json::to_string_pretty(schema).expect("render schema");
    rendered.push('\n');
    rendered
}

fn validator() -> Validator {
    Validator::new(&generate_schema_json()).expect("compile generated configuration schema")
}

fn toml_to_json(source: &str) -> Value {
    let value: toml::Value = toml::from_str(source).expect("parse test configuration as TOML");
    serde_json::to_value(value).expect("convert test configuration to JSON")
}

fn validation_errors(source: &str) -> Vec<String> {
    validator()
        .iter_errors(&toml_to_json(source))
        .map(|error| format!("{error} at {}", error.instance_path()))
        .collect()
}

fn assert_accepts(source: &str) {
    Config::parse_str(source, Path::new("arity.toml"))
        .expect("runtime configuration should accept fixture");
    let errors = validation_errors(source);
    assert!(
        errors.is_empty(),
        "schema rejected a runtime-valid configuration:\n{}",
        errors.join("\n")
    );
}

fn assert_rejects(source: &str) {
    Config::parse_str(source, Path::new("arity.toml"))
        .expect_err("runtime configuration should reject fixture");
    assert!(
        !validation_errors(source).is_empty(),
        "schema accepted a runtime-invalid configuration: {source}"
    );
}

#[test]
fn config_schema_is_in_sync() {
    let generated = render_schema(&generate_schema_json());
    let path = schema_path();

    if std::env::var_os("UPDATE_EXPECTED").is_some() {
        fs::write(&path, generated).expect("write generated configuration schema");
        return;
    }

    let checked_in = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing {}: {error}. Run `UPDATE_EXPECTED=1 cargo test config_schema` to create it.",
            path.display()
        )
    });
    similar_asserts::assert_eq!(
        checked_in,
        generated,
        "{} is out of date; run `UPDATE_EXPECTED=1 cargo test config_schema`",
        path.display()
    );
}

#[test]
fn config_schema_uses_the_public_draft_7_identity() {
    let schema = generate_schema_json();
    assert_eq!(schema["$id"], SCHEMA_ID);
    assert_eq!(schema["$schema"], "http://json-schema.org/draft-07/schema#");
    assert!(
        !render_schema(&schema).contains("\"format\": \"uint32\""),
        "schema must not expose Rust-specific formats"
    );
    validator();
}

#[test]
fn config_schema_accepts_supported_configuration() {
    assert_accepts("");
    assert_accepts(
        r#"
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
"#,
    );

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("arity.toml");
    let source = fs::read_to_string(&path).expect("read repository config");
    Config::load_from(&path).expect("runtime accepts repository config");
    let errors = validation_errors(&source);
    assert!(
        errors.is_empty(),
        "schema rejected {}:\n{}",
        path.display(),
        errors.join("\n")
    );
}

#[test]
fn config_schema_rejects_runtime_invalid_configuration() {
    for source in [
        "line-widht = 80",
        "[format]\nline-widht = 80",
        "[format]\nline-ending = \"windows\"",
        "[format]\nline-width = 0",
        "[format]\nindent-width = 1001",
        "[lint.rules]\nundesirabl-function = {}",
        "[compat]\nr = \"version four\"",
        "[index]\nremote-url = \"https://example.com\"",
        "[lint.compat]\nr = \"4.1\"",
    ] {
        assert_rejects(source);
    }
}
