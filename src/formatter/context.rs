use super::style::FormatStyle;

#[derive(Debug, Clone, Copy)]
pub(crate) struct FormatContext {
    style: FormatStyle,
}

impl FormatContext {
    pub(crate) fn new(style: FormatStyle) -> Self {
        Self { style }
    }

    pub(crate) fn style(self) -> FormatStyle {
        self.style
    }

    pub(crate) fn indent_text(self, indent: usize) -> String {
        " ".repeat(self.style.indent_width * indent)
    }
}
