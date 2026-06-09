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
}
