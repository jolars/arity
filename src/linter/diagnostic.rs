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

/// Whether a [`Fix`] preserves program behavior. `Safe` fixes are applied by
/// `lint --fix`; `Unsafe` fixes (those that could change runtime behavior, e.g.
/// deleting a statement whose RHS has side effects) require `--unsafe-fixes` or
/// an explicit editor action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Applicability {
    Safe,
    Unsafe,
}

/// A code edit that, if applied, fixes the diagnostic in question. A fix is a
/// single contiguous replacement: substitute `content` for the source bytes in
/// `start..end`.
#[derive(Debug, Clone, Serialize)]
pub struct Fix {
    /// Replacement text to substitute in.
    pub content: String,
    /// Byte offset of the start of the replacement.
    pub start: usize,
    /// Byte offset of the end of the replacement (exclusive).
    pub end: usize,
    /// Whether applying the fix preserves behavior.
    pub applicability: Applicability,
    /// Human-readable title (e.g. for an LSP code action).
    pub description: String,
}

impl Fix {
    /// A behavior-preserving fix.
    pub fn safe(
        start: usize,
        end: usize,
        content: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            content: content.into(),
            start,
            end,
            applicability: Applicability::Safe,
            description: description.into(),
        }
    }

    /// A fix that may change behavior; applied only on explicit opt-in.
    pub fn unsafe_(
        start: usize,
        end: usize,
        content: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            content: content.into(),
            start,
            end,
            applicability: Applicability::Unsafe,
            description: description.into(),
        }
    }
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
