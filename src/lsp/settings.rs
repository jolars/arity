use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ResolvedSettings {
    pub(crate) style: FormatStyle,
    pub(crate) lint: LintConfig,
    pub(crate) index: IndexConfig,
}

/// Formatter knobs the editor can push via `initializationOptions` (at startup)
/// or `workspace/didChangeConfiguration` (later). These are the *fallback*: a
/// discovered `arity.toml` is authoritative and ignores them entirely. Fields
/// are `Option` so an unset key leaves the built-in default in place.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct EditorSettings {
    line_width: Option<u32>,
    indent_width: Option<u32>,
    /// Maximum document size (bytes) for which `textDocument/documentLink` scans
    /// for file links. Documents larger than this are skipped so link discovery
    /// never walks a huge generated file. Unset uses [`DEFAULT_LINK_FILE_SIZE_LIMIT`].
    link_file_size_limit: Option<u64>,
}

/// Default cap for [`EditorSettings::link_file_size_limit`]: documents up to this
/// many bytes are scanned for document links. Sized to comfortably cover any
/// hand-written R source while excluding pathological generated files.
pub(crate) const DEFAULT_LINK_FILE_SIZE_LIMIT: u64 = 4 * 1024 * 1024;

impl EditorSettings {
    /// Extract our settings from a client-supplied JSON value. Accepts either
    /// the bare options object or a tree namespaced under a `"arity"` key (how
    /// `workspace/didChangeConfiguration` clients typically scope settings).
    /// Unknown keys are ignored, and a malformed value yields the defaults.
    pub(crate) fn from_client_value(value: &serde_json::Value) -> Self {
        let section = value
            .get("arity")
            .filter(|v| v.is_object())
            .unwrap_or(value);
        serde_json::from_value(section.clone()).unwrap_or_default()
    }

    /// The document-link size cap in bytes, falling back to
    /// [`DEFAULT_LINK_FILE_SIZE_LIMIT`] when the client left it unset.
    pub(crate) fn link_file_size_limit(&self) -> u64 {
        self.link_file_size_limit
            .unwrap_or(DEFAULT_LINK_FILE_SIZE_LIMIT)
    }

    /// The [`FormatStyle`] these settings imply, layered over the built-in
    /// defaults. Out-of-range values are rejected wholesale (falling back to
    /// defaults), reusing [`FormatConfig`]'s validation bounds — the LSP has no
    /// good channel to report a bad editor setting, so we ignore it.
    fn to_format_style(&self) -> FormatStyle {
        let mut config = FormatConfig::default();
        if let Some(width) = self.line_width {
            config.line_width = width;
        }
        if let Some(width) = self.indent_width {
            config.indent_width = width;
        }
        match config.validate(None) {
            Ok(()) => FormatStyle::from(&config),
            Err(_) => FormatStyle::default(),
        }
    }
}

/// Resolve the [`FormatStyle`] for a document: a discovered `arity.toml`
/// (`config_present`) wins outright; otherwise editor-pushed settings apply over
/// the built-in defaults.
pub(crate) fn resolve_format_style(
    config: &Config,
    config_present: bool,
    editor: &EditorSettings,
) -> FormatStyle {
    if config_present {
        FormatStyle::from(&config.format)
    } else {
        editor.to_format_style()
    }
}

#[derive(Debug)]
pub(crate) enum ConfigResolveError {
    NonFileUri,
    NoParentDirectory,
    Config(String),
}

impl std::fmt::Display for ConfigResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonFileUri => write!(f, "URI is not a file:// URI"),
            Self::NoParentDirectory => write!(f, "file has no parent directory"),
            Self::Config(msg) => f.write_str(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- editor settings --------------------------------------------------

    #[test]
    fn editor_settings_parse_bare_camel_case_object() {
        let value = serde_json::json!({ "lineWidth": 100, "indentWidth": 4 });
        let settings = EditorSettings::from_client_value(&value);
        assert_eq!(settings.line_width, Some(100));
        assert_eq!(settings.indent_width, Some(4));
    }

    #[test]
    fn editor_settings_parse_namespaced_under_arity() {
        // didChangeConfiguration clients push their whole settings tree; ours is
        // scoped under "arity" and sibling keys are ignored.
        let value = serde_json::json!({
            "arity": { "lineWidth": 120 },
            "editor": { "tabSize": 8 },
        });
        let settings = EditorSettings::from_client_value(&value);
        assert_eq!(settings.line_width, Some(120));
        assert_eq!(settings.indent_width, None);
    }

    #[test]
    fn editor_settings_ignore_unknown_and_malformed() {
        let unknown = serde_json::json!({ "bogus": true });
        assert_eq!(
            EditorSettings::from_client_value(&unknown),
            EditorSettings::default()
        );
        let malformed = serde_json::json!("not an object");
        assert_eq!(
            EditorSettings::from_client_value(&malformed),
            EditorSettings::default()
        );
    }

    #[test]
    fn editor_settings_parse_link_file_size_limit() {
        let value = serde_json::json!({ "linkFileSizeLimit": 8192 });
        let settings = EditorSettings::from_client_value(&value);
        assert_eq!(settings.link_file_size_limit(), 8192);
    }

    #[test]
    fn editor_settings_link_file_size_limit_defaults_when_unset() {
        let settings = EditorSettings::default();
        assert_eq!(
            settings.link_file_size_limit(),
            DEFAULT_LINK_FILE_SIZE_LIMIT
        );
    }

    #[test]
    fn editor_settings_to_style_layers_over_defaults() {
        let settings = EditorSettings {
            line_width: Some(100),
            indent_width: None,
            ..Default::default()
        };
        let style = settings.to_format_style();
        assert_eq!(style.line_width, 100);
        // Unset field keeps the built-in default.
        assert_eq!(style.indent_width, FormatStyle::default().indent_width);
    }

    #[test]
    fn editor_settings_out_of_range_fall_back_to_defaults() {
        // 0 is below the valid width floor; the whole layer is discarded.
        let settings = EditorSettings {
            line_width: Some(0),
            indent_width: Some(4),
            ..Default::default()
        };
        assert_eq!(settings.to_format_style(), FormatStyle::default());
    }

    #[test]
    fn config_file_wins_over_editor_settings() {
        let mut config = Config::default();
        config.format.line_width = 70;
        let editor = EditorSettings {
            line_width: Some(120),
            indent_width: Some(8),
            ..Default::default()
        };
        // arity.toml present → editor settings ignored entirely.
        let style = resolve_format_style(&config, true, &editor);
        assert_eq!(style.line_width, 70);
        assert_eq!(style.indent_width, FormatStyle::default().indent_width);
        // No config file → editor settings apply over defaults.
        let fallback = resolve_format_style(&Config::default(), false, &editor);
        assert_eq!(fallback.line_width, 120);
        assert_eq!(fallback.indent_width, 8);
    }
}
