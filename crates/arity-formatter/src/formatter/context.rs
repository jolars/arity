use rowan::TextRange;

use super::style::FormatStyle;

#[derive(Debug, Clone, Copy)]
pub(crate) struct FormatContext {
    style: FormatStyle,
    ignored_directive: Option<TextRange>,
}

impl FormatContext {
    pub(crate) fn new(style: FormatStyle) -> Self {
        Self {
            style,
            ignored_directive: None,
        }
    }

    pub(crate) fn ignoring_directive(style: FormatStyle, range: TextRange) -> Self {
        Self {
            style,
            ignored_directive: Some(range),
        }
    }

    pub(crate) fn style(self) -> FormatStyle {
        self.style
    }

    pub(crate) fn ignored_directive(self) -> Option<TextRange> {
        self.ignored_directive
    }
}
