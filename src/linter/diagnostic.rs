//! Diagnostic, Fix, and Violation types — jarl-aligned shape.

use std::path::PathBuf;

use rowan::TextRange;
use serde::Serialize;

/// Severity levels for a diagnostic. Mirrors LSP's severity enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

/// A code edit that, if applied, fixes the diagnostic in question. Carried in
/// `Diagnostic::fix` for forward compatibility; no rules emit fixes yet, and
/// the `--fix` CLI flag is not implemented in this pass.
#[derive(Debug, Clone, Serialize)]
pub struct Fix {
    /// Replacement text to substitute in.
    pub content: String,
    /// Byte offset of the start of the replacement.
    pub start: usize,
    /// Byte offset of the end of the replacement (exclusive).
    pub end: usize,
}

/// Render-ready violation metadata that the renderer consumes. `name` is the
/// short name (typically the rule ID); `body` is a one-line explanation;
/// `suggestion` is an optional follow-on hint.
#[derive(Debug, Clone, Serialize)]
pub struct ViolationData {
    pub name: String,
    pub body: String,
    pub suggestion: Option<String>,
}

impl ViolationData {
    pub fn new(name: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            body: body.into(),
            suggestion: None,
        }
    }

    pub fn with_suggestion(mut self, hint: impl Into<String>) -> Self {
        self.suggestion = Some(hint.into());
        self
    }
}

/// A lint finding.
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    /// Static rule ID (e.g. `"unused-binding"`).
    pub rule: &'static str,
    pub severity: Severity,
    pub path: PathBuf,
    /// Source range, in bytes.
    #[serde(serialize_with = "serialize_text_range")]
    pub range: TextRange,
    pub message: ViolationData,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
}

fn serialize_text_range<S: serde::Serializer>(
    range: &TextRange,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeStruct;
    let mut s = serializer.serialize_struct("Range", 2)?;
    s.serialize_field("start", &u32::from(range.start()))?;
    s.serialize_field("end", &u32::from(range.end()))?;
    s.end()
}

/// Trait implemented by per-rule violation structs. Rules construct one of
/// these and convert to a [`Diagnostic`] via `Rule::report`.
pub trait Violation {
    /// Short name (usually the rule ID).
    fn name(&self) -> String;
    /// One-line body explaining what's wrong.
    fn body(&self) -> String;
    /// Optional follow-on suggestion.
    fn suggestion(&self) -> Option<String> {
        None
    }
}

impl<T: Violation> From<&T> for ViolationData {
    fn from(value: &T) -> Self {
        Self {
            name: value.name(),
            body: value.body(),
            suggestion: value.suggestion(),
        }
    }
}
