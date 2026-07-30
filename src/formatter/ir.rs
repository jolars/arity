//! A lightweight Wadler/Prettier-style intermediate representation (IR) for the
//! formatter.
//!
//! Construct formatters build an [`Ir`] tree describing *possible* layouts (with
//! break-points), and [`super::printer::Printer`] resolves it against the
//! configured line width into a final string. This replaces the older model
//! where each construct rendered directly to a `String` and width was measured
//! retrospectively.

// The IR exposes a complete primitive vocabulary. A few builders
// (`group_expanded`, `verbatim_forced`, `join`) are not yet exercised because
// the heaviest arg-list constructs (subset/call/function) still use their
// specialized string renderers; they are kept for the planned native IR
// arg-wrapping work.
#![allow(dead_code)]

use std::rc::Rc;

/// A document node describing how a piece of code may be laid out.
#[derive(Debug, Clone)]
pub(crate) enum Ir {
    /// Literal text. Must never contain a newline.
    Text(Rc<str>),
    /// A sequence of nodes printed back-to-back.
    Concat(Rc<[Ir]>),
    /// Flat mode: a single space. Break mode: newline + current indent.
    Line,
    /// Flat mode: nothing. Break mode: newline + current indent.
    SoftLine,
    /// Always a newline + current indent, regardless of mode. Forces every
    /// enclosing [`Ir::Group`] to break.
    HardLine,
    /// A blank line followed by the next line's indent. Like [`Ir::HardLine`] it
    /// forces enclosing groups to break.
    EmptyLine,
    /// Increase the indent of everything inside by one `indent_width` step.
    Indent(Rc<Ir>),
    /// Render `inner` in break mode regardless of the enclosing mode, while
    /// staying *transparent* to flat fit-measurement (unlike [`Ir::Group`] with
    /// `expand`, it never forces a parent's fit check to fail by itself).
    ///
    /// This exists for the body of a trailing block that hugs its prefix: a hug
    /// [`Ir::Group`] is kept flat as long as the prefix up to the opening brace
    /// fits, so its `inner` is rendered in `Mode::Flat` — but the block *body*
    /// past that brace always lays out multi-line (its statements are separated
    /// by [`Ir::HardLine`]s). Leaving the body in the hug's flat mode would
    /// violate the invariant that a subtree containing a hard break is laid out
    /// broken, and would poison the printer's rest-of-line fit measurement: a
    /// breakable group inside the body would be mis-measured flat (see
    /// `Printer::rest_fits`). Wrapping the body in `BreakBody` re-establishes the
    /// break mode the body genuinely renders in.
    BreakBody(Rc<Ir>),
    /// A break-decision boundary. The printer measures the flat rendering of
    /// `inner`; if it fits and contains no forced break, it prints flat,
    /// otherwise broken. `expand` forces broken unconditionally.
    ///
    /// `hug` enables trailing-block hugging: the fit measurement stops
    /// *successfully* at the first forced line break (the opening of a trailing
    /// block) rather than failing on it. This lets a group whose last element is
    /// a block (`f(a, {`…`})`) stay flat — the prefix hugs the block's open
    /// brace — when only the prefix needs to fit. A comment in the prefix
    /// (`Verbatim { force_break: true }`) still fails the fit, forcing expansion.
    Group {
        inner: Rc<Ir>,
        expand: bool,
        hug: bool,
        /// Only meaningful together with `hug`. When set, the prefix fit
        /// measurement *excuses* a leading argument that is an unbreakable atom
        /// too wide to fit on any line (`width >= line_width`): such an atom
        /// would overflow whether or not the list breaks, so it must not, by
        /// itself, force the hug to expand. Set by the rule only when every
        /// leading argument is such a bare atom (no nested breakable group, so
        /// nothing is rescuable by breaking). See the `test_that("<long>", {…})`
        /// case: breaking buys no width, only lines.
        hug_excuse_overflow: bool,
    },
    /// Emit `flat` when the enclosing group is flat, `broken` when it is broken.
    IfBreak { flat: Rc<Ir>, broken: Rc<Ir> },
    /// Pre-rendered text (comments, or not-yet-migrated constructs) spliced
    /// through untouched. When `force_break` is set the enclosing group cannot
    /// stay flat (used for comments and for multi-line bridged renderings);
    /// otherwise it behaves as opaque inline text of its own width.
    Verbatim { text: Rc<str>, force_break: bool },
    /// A trailing comment that runs to end of line, rendered inline where it sits
    /// but counted as **zero width** by every fit measurement. This is the
    /// Wadler/Prettier "line suffix" concept, scoped to same-line trailing
    /// comments: air treats such a comment as a zero-width suffix, so a long
    /// comment never forces an otherwise-fitting group to break. The break that
    /// must follow a trailing comment (no code can share its line) is supplied
    /// separately by a following hard break (an adjacent [`Ir::HardLine`], or the
    /// statement separator between lines), so this node itself does not force a
    /// break. The text must not contain a newline.
    LineSuffix(Rc<str>),
    /// An ordered list of candidate layouts. The printer picks the first
    /// candidate whose *first line* fits at the current column under a
    /// break-aware measurement (nested groups decide their own break, success
    /// is the first emitted newline); if none fit, the last candidate is
    /// rendered broken. With a single candidate this degenerates to a
    /// "break-aware group": flat if its first line fits, broken otherwise.
    /// Must contain at least one candidate.
    ConditionalGroup(Rc<[Ir]>),
    /// Same shape as [`Ir::ConditionalGroup`] but selected by an *all-lines*
    /// measurement: the printer renders each candidate at the current column
    /// and picks the first whose every rendered line fits within
    /// `line_width`. The last candidate is rendered broken when none fit.
    /// Use for choices like "keep this body bare if every rendered line fits,
    /// else wrap in braces" — the IR port of the legacy `fits_with_newlines`
    /// check.
    ConditionalGroupAllLines(Rc<[Ir]>),
    /// Nothing.
    Nil,
}

impl Ir {
    pub(crate) fn text(s: impl Into<Rc<str>>) -> Ir {
        Ir::Text(s.into())
    }

    pub(crate) fn concat(items: impl IntoIterator<Item = Ir>) -> Ir {
        let items: Vec<Ir> = items
            .into_iter()
            .filter(|i| !matches!(i, Ir::Nil))
            .collect();
        match items.len() {
            0 => Ir::Nil,
            1 => items.into_iter().next().unwrap(),
            _ => Ir::Concat(items.into()),
        }
    }

    /// Interleave `items` with `sep`.
    pub(crate) fn join(sep: Ir, items: impl IntoIterator<Item = Ir>) -> Ir {
        let mut out = Vec::new();
        for (i, item) in items.into_iter().enumerate() {
            if i > 0 {
                out.push(sep.clone());
            }
            out.push(item);
        }
        Ir::concat(out)
    }

    pub(crate) fn group(inner: Ir) -> Ir {
        Ir::Group {
            inner: Rc::new(inner),
            expand: false,
            hug: false,
            hug_excuse_overflow: false,
        }
    }

    pub(crate) fn group_expanded(inner: Ir) -> Ir {
        Ir::Group {
            inner: Rc::new(inner),
            expand: true,
            hug: false,
            hug_excuse_overflow: false,
        }
    }

    /// A group that hugs a trailing block: the printer keeps it flat as long as
    /// the prefix up to the block's opening brace fits, then lets the block
    /// break onto its own lines. See [`Ir::Group`]'s `hug` field.
    pub(crate) fn group_hug(inner: Ir) -> Ir {
        Ir::Group {
            inner: Rc::new(inner),
            expand: false,
            hug: true,
            hug_excuse_overflow: false,
        }
    }

    /// Like [`Self::group_hug`], but the prefix fit measurement excuses a
    /// leading argument that is an unbreakable atom too wide to fit on any line.
    /// See [`Ir::Group`]'s `hug_excuse_overflow` field. Callers must only use
    /// this when every leading argument is a bare atom (nothing breaking could
    /// rescue), so the excuse cannot hide a genuinely fittable argument.
    pub(crate) fn group_hug_excused(inner: Ir) -> Ir {
        Ir::Group {
            inner: Rc::new(inner),
            expand: false,
            hug: true,
            hug_excuse_overflow: true,
        }
    }

    /// An ordered list of candidate layouts; see [`Ir::ConditionalGroup`].
    /// Panics if `candidates` is empty.
    pub(crate) fn conditional_group(candidates: impl IntoIterator<Item = Ir>) -> Ir {
        let cands: Vec<Ir> = candidates.into_iter().collect();
        assert!(
            !cands.is_empty(),
            "Ir::conditional_group requires at least one candidate"
        );
        Ir::ConditionalGroup(cands.into())
    }

    /// An ordered list of candidate layouts selected by all-lines-fit; see
    /// [`Ir::ConditionalGroupAllLines`]. Panics if `candidates` is empty.
    pub(crate) fn conditional_group_all_lines(candidates: impl IntoIterator<Item = Ir>) -> Ir {
        let cands: Vec<Ir> = candidates.into_iter().collect();
        assert!(
            !cands.is_empty(),
            "Ir::conditional_group_all_lines requires at least one candidate"
        );
        Ir::ConditionalGroupAllLines(cands.into())
    }

    pub(crate) fn indent(inner: Ir) -> Ir {
        Ir::Indent(Rc::new(inner))
    }

    /// Render `inner` in break mode, transparent to flat fit-measurement; see
    /// [`Ir::BreakBody`].
    pub(crate) fn break_body(inner: Ir) -> Ir {
        Ir::BreakBody(Rc::new(inner))
    }

    pub(crate) fn if_break(flat: Ir, broken: Ir) -> Ir {
        Ir::IfBreak {
            flat: Rc::new(flat),
            broken: Rc::new(broken),
        }
    }

    /// A bridged/inline verbatim chunk. It forces a break only if it spans
    /// multiple lines (i.e. its own layout cannot be collapsed).
    pub(crate) fn verbatim(s: impl Into<Rc<str>>) -> Ir {
        let text: Rc<str> = s.into();
        let force_break = text.contains('\n');
        Ir::Verbatim { text, force_break }
    }

    /// A verbatim chunk that always forces the enclosing group to break,
    /// regardless of whether it spans multiple lines (e.g. a comment).
    pub(crate) fn verbatim_forced(s: impl Into<Rc<str>>) -> Ir {
        Ir::Verbatim {
            text: s.into(),
            force_break: true,
        }
    }

    /// A trailing comment rendered inline but measured as zero width; see
    /// [`Ir::LineSuffix`].
    pub(crate) fn line_suffix(s: impl Into<Rc<str>>) -> Ir {
        Ir::LineSuffix(s.into())
    }

    pub(crate) fn line() -> Ir {
        Ir::Line
    }

    pub(crate) fn soft_line() -> Ir {
        Ir::SoftLine
    }

    /// Whether this tree contains a nested breakable group (`Group` or either
    /// `ConditionalGroup` variant). Used by the arg-hug rule to decide whether a
    /// leading argument is a bare atom: if it holds a breakable group, its
    /// overflow may be rescuable by breaking, so the hug must not excuse it.
    pub(crate) fn contains_group(&self) -> bool {
        match self {
            Ir::Group { .. } | Ir::ConditionalGroup(_) | Ir::ConditionalGroupAllLines(_) => true,
            Ir::Concat(items) => items.iter().any(Ir::contains_group),
            Ir::Indent(inner) | Ir::BreakBody(inner) => inner.contains_group(),
            Ir::IfBreak { flat, broken } => flat.contains_group() || broken.contains_group(),
            Ir::Text(_)
            | Ir::Verbatim { .. }
            | Ir::LineSuffix(_)
            | Ir::HardLine
            | Ir::EmptyLine
            | Ir::Line
            | Ir::SoftLine
            | Ir::Nil => false,
        }
    }

    pub(crate) fn hard_line() -> Ir {
        Ir::HardLine
    }

    pub(crate) fn empty_line() -> Ir {
        Ir::EmptyLine
    }

    pub(crate) fn nil() -> Ir {
        Ir::Nil
    }

    /// Whether this tree contains an *unconditional* forced line break: a
    /// `HardLine`/`EmptyLine`, a force-break `Verbatim` (e.g. a comment), or an
    /// `expand` group. Conditional breaks (`IfBreak` branches, `SoftLine`,
    /// `Line`) do not count, since they only break when an enclosing group does.
    /// Used to detect, e.g., a non-empty block argument that should force its
    /// arg list open.
    pub(crate) fn contains_forced_break(&self) -> bool {
        match self {
            Ir::HardLine | Ir::EmptyLine => true,
            Ir::Verbatim { force_break, .. } => *force_break,
            Ir::Concat(items) => items.iter().any(Ir::contains_forced_break),
            Ir::Indent(inner) | Ir::BreakBody(inner) => inner.contains_forced_break(),
            Ir::Group { inner, expand, .. } => *expand || inner.contains_forced_break(),
            // The flat-most candidate decides: if even it forces a break, the
            // conditional group always breaks; otherwise some layout is flat-able.
            Ir::ConditionalGroup(cands) | Ir::ConditionalGroupAllLines(cands) => {
                cands.first().is_some_and(Ir::contains_forced_break)
            }
            Ir::Text(_)
            | Ir::LineSuffix(_)
            | Ir::Line
            | Ir::SoftLine
            | Ir::IfBreak { .. }
            | Ir::Nil => false,
        }
    }

    /// Whether the document ends on a trailing comment that runs to end of
    /// line: an [`Ir::LineSuffix`], or a `Verbatim` splice whose last line is a
    /// comment. Anything a caller appends after such a document lands *inside*
    /// the comment and is swallowed, so a caller that still owes a closing
    /// delimiter must emit a break first (or choose a broken layout).
    ///
    /// Conservative on the branching nodes: any candidate/branch that could be
    /// selected counts, since the layout choice is the printer's to make.
    pub(crate) fn ends_with_line_suffix(&self) -> bool {
        match self {
            Ir::LineSuffix(_) => true,
            Ir::Verbatim { text, .. } => text
                .rsplit('\n')
                .next()
                .is_some_and(|last| last.trim_start().starts_with('#')),
            Ir::Concat(items) => items
                .iter()
                .rev()
                .find(|item| !matches!(item, Ir::Nil))
                .is_some_and(Ir::ends_with_line_suffix),
            Ir::Indent(inner) | Ir::BreakBody(inner) | Ir::Group { inner, .. } => {
                inner.ends_with_line_suffix()
            }
            Ir::IfBreak { flat, broken } => {
                flat.ends_with_line_suffix() || broken.ends_with_line_suffix()
            }
            Ir::ConditionalGroup(cands) | Ir::ConditionalGroupAllLines(cands) => {
                cands.iter().any(Ir::ends_with_line_suffix)
            }
            Ir::Text(_) | Ir::HardLine | Ir::EmptyLine | Ir::Line | Ir::SoftLine | Ir::Nil => false,
        }
    }
}
