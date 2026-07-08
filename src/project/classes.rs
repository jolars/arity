//! Per-file OOP class projection: the classes a file declares and their
//! supertype (inheritance) edges, for S4 (`setClass`), R6 (`R6Class`), and
//! reference classes (`setRefClass`).
//!
//! The class-hierarchy analog of [`file_def_sites`](super::exports::file_def_sites):
//! a **range-free** CST projection that backs the cross-file class index
//! ([`project_classes`](super::graph::project_classes)) and, through it, LSP type
//! hierarchy. Range-free so it backdates across a body edit exactly like
//! [`file_exports`](super::exports::file_exports); a consumer recovers a class's
//! actual span per request via [`locate_class_def`].
//!
//! Inheritance edges are keyed on the declared **string** class name (the first
//! positional string argument). S4/reference-class parents come from `contains=`
//! (a string or a `c(...)` of strings); R6's `inherit=` names a generator
//! *symbol*, so v1 uses that identifier's text as the parent class name — this
//! relies on the near-universal convention that `Bar <- R6Class("Bar", ...)`
//! binds the generator under its own class name.

use std::collections::BTreeMap;

use rowan::ast::AstNode as _;
use rowan::{NodeOrToken, TextRange, TextSize, TokenAtOffset};

use crate::ast::{AssignmentExpr, CallExpr};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

/// Which R OOP system a class definition belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, salsa::Update)]
pub enum ClassSystem {
    /// `setClass("X", contains = ...)`.
    S4,
    /// `R6Class("X", inherit = ...)`.
    R6,
    /// `setRefClass("X", contains = ...)`.
    RefClass,
}

impl ClassSystem {
    /// A short human label for a hover/`detail` string.
    pub fn label(self) -> &'static str {
        match self {
            ClassSystem::S4 => "S4 class",
            ClassSystem::R6 => "R6 class",
            ClassSystem::RefClass => "reference class",
        }
    }

    /// The system a defining call's callee name denotes, if any.
    fn from_callee(name: &str) -> Option<Self> {
        match name {
            "setClass" => Some(ClassSystem::S4),
            "R6Class" => Some(ClassSystem::R6),
            "setRefClass" => Some(ClassSystem::RefClass),
            _ => None,
        }
    }
}

/// One class definition: its system and the declared supertype names.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, salsa::Update)]
pub struct ClassDef {
    pub system: ClassSystem,
    pub parents: Vec<String>,
}

/// Every S4/R6/reference class defined anywhere in `root`, keyed by declared
/// class name and tagged with its system and declared supertypes.
///
/// **Range-free** (names only) so it backdates like [`file_def_sites`]; recover a
/// class's span per request via [`locate_class_def`]. When a name is defined more
/// than once, the first definition (in tree order) wins.
pub fn file_class_defs(root: &SyntaxNode) -> BTreeMap<String, ClassDef> {
    let mut defs: BTreeMap<String, ClassDef> = BTreeMap::new();
    for node in root.descendants() {
        if let Some((name, def)) = class_def_at(&node) {
            defs.entry(name).or_insert(def);
        }
    }
    defs
}

/// The class defined by the call at `node`, if it is a `setClass`/`R6Class`/
/// `setRefClass` call with a string class name.
fn class_def_at(node: &SyntaxNode) -> Option<(String, ClassDef)> {
    let call = CallExpr::cast(node.clone())?;
    let system = class_def_system(&call)?;
    let (_, name) = class_name_token(&call)?;
    let parents = match system {
        ClassSystem::S4 | ClassSystem::RefClass => contains_parents(&call),
        ClassSystem::R6 => inherit_parent(&call).into_iter().collect(),
    };
    Some((name, ClassDef { system, parents }))
}

/// The class span for `name` in `root`, recovered from the fresh tree: the
/// system, the class-name string span (a selection range), and the enclosing
/// top-level statement span. `None` when no class of that name is defined here.
pub fn locate_class_def(root: &SyntaxNode, name: &str) -> Option<ClassLocation> {
    for node in root.descendants() {
        let Some(call) = CallExpr::cast(node.clone()) else {
            continue;
        };
        let Some(system) = class_def_system(&call) else {
            continue;
        };
        let Some((token, value)) = class_name_token(&call) else {
            continue;
        };
        if value != name {
            continue;
        }
        let full = top_level_statement(root, node.text_range())?;
        return Some(ClassLocation {
            system,
            name_range: token.text_range(),
            full_range: full,
        });
    }
    None
}

/// The located span of a class definition (see [`locate_class_def`]).
#[derive(Debug, Clone)]
pub struct ClassLocation {
    pub system: ClassSystem,
    /// The class-name string token span — the item's selection range.
    pub name_range: TextRange,
    /// The enclosing top-level statement span — the item's full range.
    pub full_range: TextRange,
}

/// The class name the cursor at `offset` denotes, for `prepareTypeHierarchy`:
/// a class-name or `contains=` string in a class-def call, the binding
/// identifier of `X <- R6Class(...)`, an `inherit =` identifier, or any bare
/// identifier (resolved cross-file by the caller). `None` when the cursor names
/// nothing class-like.
pub fn class_name_at_offset(root: &SyntaxNode, offset: TextSize) -> Option<String> {
    let token = token_at(root, offset)?;
    match token.kind() {
        SyntaxKind::STRING => {
            // A string is a class name only inside a class-def call: its own
            // value is the defined class (the name arg) or a `contains=` parent.
            class_def_call_around_string(&token).map(|_| token_text_unquoted(&token))
        }
        SyntaxKind::IDENT => {
            // A binding `X <- <classdefcall>` resolves to the class defined there
            // (which may differ from `X`); otherwise the identifier text is a
            // class name resolved locally or cross-file by the caller (this also
            // covers an `inherit = Ident` value).
            Some(ident_defines_class(&token).unwrap_or_else(|| token.text().to_string()))
        }
        _ => None,
    }
}

// --- extraction helpers ----------------------------------------------------

/// The system of `call` when it is a `setClass`/`R6Class`/`setRefClass` call
/// with a simple identifier callee.
fn class_def_system(call: &CallExpr) -> Option<ClassSystem> {
    let callee = call.callee_token()?;
    (callee.kind() == SyntaxKind::IDENT)
        .then(|| ClassSystem::from_callee(callee.text()))
        .flatten()
}

/// The first positional string argument of `call` — the declared class name —
/// as its token and unquoted value.
fn class_name_token(call: &CallExpr) -> Option<(SyntaxToken, String)> {
    let value = call_args(call)
        .into_iter()
        .find(|a| a.name.is_none())?
        .value?;
    match value {
        NodeOrToken::Token(t) if t.kind() == SyntaxKind::STRING => {
            let unquoted = token_text_unquoted(&t);
            Some((t, unquoted))
        }
        _ => None,
    }
}

/// The supertype names from a `contains=` argument (a string, or a `c(...)` of
/// strings). Empty when absent.
fn contains_parents(call: &CallExpr) -> Vec<String> {
    call_args(call)
        .into_iter()
        .find(|a| a.name.as_deref() == Some("contains"))
        .and_then(|a| a.value)
        .map(|v| string_list(&v))
        .unwrap_or_default()
}

/// The single supertype name from an R6 `inherit=` argument (a generator
/// identifier). `None` when absent or not a bare identifier.
fn inherit_parent(call: &CallExpr) -> Option<String> {
    let value = call_args(call)
        .into_iter()
        .find(|a| a.name.as_deref() == Some("inherit"))?
        .value?;
    match value {
        NodeOrToken::Token(t) if t.kind() == SyntaxKind::IDENT => Some(t.text().to_string()),
        _ => None,
    }
}

/// The string names in `element`: one for a bare string, several for a `c(...)`
/// call of strings, none otherwise.
fn string_list(element: &SyntaxElement) -> Vec<String> {
    match element {
        NodeOrToken::Token(t) if t.kind() == SyntaxKind::STRING => vec![token_text_unquoted(t)],
        NodeOrToken::Node(n) => match CallExpr::cast(n.clone()) {
            Some(call) if call.callee_token().is_some_and(|t| t.text() == "c") => call_args(&call)
                .into_iter()
                .filter_map(|a| match a.value? {
                    NodeOrToken::Token(t) if t.kind() == SyntaxKind::STRING => {
                        Some(token_text_unquoted(&t))
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// One call argument split into its optional name (for `name = value`) and its
/// value element.
struct CallArg {
    name: Option<String>,
    value: Option<SyntaxElement>,
}

/// The arguments of `call`, each split into name and value.
fn call_args(call: &CallExpr) -> Vec<CallArg> {
    let Some(list) = call.arg_list() else {
        return Vec::new();
    };
    list.args().map(|arg| split_arg(arg.syntax())).collect()
}

fn split_arg(arg: &SyntaxNode) -> CallArg {
    let elements: Vec<SyntaxElement> = arg.children_with_tokens().collect();
    match elements
        .iter()
        .position(|e| e.kind() == SyntaxKind::ASSIGN_EQ)
    {
        Some(eq) => {
            let name = elements[..eq].iter().rev().find_map(|e| match e {
                NodeOrToken::Token(t)
                    if matches!(t.kind(), SyntaxKind::IDENT | SyntaxKind::STRING) =>
                {
                    Some(token_text_unquoted(t))
                }
                _ => None,
            });
            let value = elements[eq + 1..]
                .iter()
                .find(|e| !is_trivia(e.kind()))
                .cloned();
            CallArg { name, value }
        }
        None => CallArg {
            name: None,
            value: elements.iter().find(|e| !is_trivia(e.kind())).cloned(),
        },
    }
}

// --- cursor helpers --------------------------------------------------------

/// The class-def call a string token names a class within: its directly
/// enclosing call when that is a class-def call, or the class-def call wrapping
/// a `c(...)` the string sits in (a `contains=` vector). `None` otherwise.
fn class_def_call_around_string(token: &SyntaxToken) -> Option<CallExpr> {
    let call = enclosing_call(token.parent()?)?;
    if class_def_system(&call).is_some() {
        return Some(call);
    }
    if call.callee_token().is_some_and(|t| t.text() == "c") {
        let outer = enclosing_call(call.syntax().parent()?)?;
        if class_def_system(&outer).is_some() {
            return Some(outer);
        }
    }
    None
}

/// The class an identifier binding target defines, when it is the target of
/// `X <- <class-def call>` — the call's declared string name, falling back to
/// the identifier text. `None` when the identifier is not such a target.
fn ident_defines_class(token: &SyntaxToken) -> Option<String> {
    let assign = token.parent()?.ancestors().find_map(AssignmentExpr::cast)?;
    if assign.target_name_token()?.text_range() != token.text_range() {
        return None;
    }
    let NodeOrToken::Node(value) = assign.value_element()? else {
        return None;
    };
    let call = CallExpr::cast(value)?;
    class_def_system(&call)?;
    Some(
        class_name_token(&call)
            .map(|(_, v)| v)
            .unwrap_or_else(|| token.text().to_string()),
    )
}

/// The nearest `CALL_EXPR` ancestor of `node` (inclusive).
fn enclosing_call(node: SyntaxNode) -> Option<CallExpr> {
    node.ancestors().find_map(CallExpr::cast)
}

/// The root-level statement span containing `range`.
fn top_level_statement(root: &SyntaxNode, range: TextRange) -> Option<TextRange> {
    root.children()
        .find(|c| c.text_range().contains_range(range))
        .map(|c| c.text_range())
}

/// The token to interpret at `offset`, preferring an `IDENT`/`STRING` when the
/// offset falls between two tokens.
fn token_at(root: &SyntaxNode, offset: TextSize) -> Option<SyntaxToken> {
    match root.token_at_offset(offset) {
        TokenAtOffset::None => None,
        TokenAtOffset::Single(t) => Some(t),
        TokenAtOffset::Between(left, right) => {
            let named = |k| matches!(k, SyntaxKind::IDENT | SyntaxKind::STRING);
            if named(right.kind()) {
                Some(right)
            } else if named(left.kind()) {
                Some(left)
            } else {
                Some(right)
            }
        }
    }
}

fn is_trivia(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
    )
}

/// The unquoted contents of a backtick/quoted `STRING` token, else its raw text.
fn token_text_unquoted(token: &SyntaxToken) -> String {
    let text = token.text();
    let bytes = text.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if matches!(first, b'"' | b'\'' | b'`') && first == last {
            return text[1..text.len() - 1].to_string();
        }
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn defs_of(src: &str) -> BTreeMap<String, ClassDef> {
        file_class_defs(&parse(src).cst)
    }

    fn parents_of<'a>(defs: &'a BTreeMap<String, ClassDef>, name: &str) -> &'a [String] {
        &defs.get(name).expect("class def").parents
    }

    #[test]
    fn s4_contains_single_string() {
        let defs = defs_of("setClass(\"Child\", contains = \"Parent\")\n");
        assert_eq!(defs["Child"].system, ClassSystem::S4);
        assert_eq!(parents_of(&defs, "Child"), ["Parent"]);
    }

    #[test]
    fn s4_contains_vector() {
        let defs = defs_of("setClass(\"C\", contains = c(\"A\", \"B\"))\n");
        assert_eq!(parents_of(&defs, "C"), ["A", "B"]);
    }

    #[test]
    fn s4_without_contains_has_no_parents() {
        let defs = defs_of("setClass(\"Root\")\n");
        assert_eq!(defs["Root"].system, ClassSystem::S4);
        assert!(parents_of(&defs, "Root").is_empty());
    }

    #[test]
    fn r6_inherit_identifier() {
        let defs = defs_of("Child <- R6Class(\"Child\", inherit = Parent)\n");
        assert_eq!(defs["Child"].system, ClassSystem::R6);
        assert_eq!(parents_of(&defs, "Child"), ["Parent"]);
    }

    #[test]
    fn r6_without_inherit_has_no_parents() {
        let defs = defs_of("Animal <- R6Class(\"Animal\", public = list())\n");
        assert_eq!(defs["Animal"].system, ClassSystem::R6);
        assert!(parents_of(&defs, "Animal").is_empty());
    }

    #[test]
    fn refclass_contains_string() {
        let defs = defs_of("Child <- setRefClass(\"Child\", contains = \"Parent\")\n");
        assert_eq!(defs["Child"].system, ClassSystem::RefClass);
        assert_eq!(parents_of(&defs, "Child"), ["Parent"]);
    }

    #[test]
    fn single_quoted_class_name_is_unquoted() {
        let defs = defs_of("setClass('Child', contains = 'Parent')\n");
        assert_eq!(parents_of(&defs, "Child"), ["Parent"]);
    }

    #[test]
    fn non_class_calls_are_ignored() {
        let defs = defs_of("print(\"hello\")\nx <- list(contains = \"y\")\n");
        assert!(defs.is_empty());
    }

    #[test]
    fn locate_yields_name_and_statement_spans() {
        let src = "Child <- R6Class(\"Child\", inherit = Parent)\n";
        let root = parse(src).cst;
        let loc = locate_class_def(&root, "Child").expect("located");
        assert_eq!(loc.system, ClassSystem::R6);
        // Selection is the class-name string literal (quotes included).
        assert_eq!(&src[loc.name_range], "\"Child\"");
        // Full range is the whole assignment statement (sans trailing newline).
        assert_eq!(
            &src[loc.full_range],
            "Child <- R6Class(\"Child\", inherit = Parent)"
        );
    }

    fn name_at(src: &str, needle: &str) -> Option<String> {
        let root = parse(src).cst;
        let offset = TextSize::new(src.find(needle).expect("needle") as u32 + 1);
        class_name_at_offset(&root, offset)
    }

    #[test]
    fn cursor_on_class_name_string() {
        assert_eq!(
            name_at("setClass(\"Child\", contains = \"Parent\")\n", "\"Child\""),
            Some("Child".to_string())
        );
    }

    #[test]
    fn cursor_on_contains_string() {
        assert_eq!(
            name_at("setClass(\"Child\", contains = \"Parent\")\n", "\"Parent\""),
            Some("Parent".to_string())
        );
    }

    #[test]
    fn cursor_on_contains_vector_element() {
        assert_eq!(
            name_at("setClass(\"C\", contains = c(\"A\", \"B\"))\n", "\"B\""),
            Some("B".to_string())
        );
    }

    #[test]
    fn cursor_on_r6_binding_identifier() {
        assert_eq!(
            name_at(
                "Child <- R6Class(\"Child\", inherit = Parent)\n",
                "Child <-"
            ),
            Some("Child".to_string())
        );
    }

    #[test]
    fn cursor_on_inherit_identifier() {
        assert_eq!(
            name_at("Child <- R6Class(\"Child\", inherit = Parent)\n", "Parent)"),
            Some("Parent".to_string())
        );
    }

    #[test]
    fn cursor_on_plain_string_names_nothing() {
        assert_eq!(name_at("x <- \"Parent\"\n", "\"Parent\""), None);
    }
}
