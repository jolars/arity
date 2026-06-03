use rowan::ast::support;
use rowan::{SyntaxElement, SyntaxToken};
use smol_str::SmolStr;

use crate::ast::AstNode;
use crate::syntax::{RLanguage, SyntaxKind, SyntaxNode};

macro_rules! ast_node {
    ($name:ident, $kind:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(SyntaxNode);

        impl AstNode for $name {
            type Language = RLanguage;

            fn can_cast(kind: SyntaxKind) -> bool {
                kind == $kind
            }

            fn cast(syntax: SyntaxNode) -> Option<Self> {
                Self::can_cast(syntax.kind()).then(|| Self(syntax))
            }

            fn syntax(&self) -> &SyntaxNode {
                &self.0
            }
        }
    };
}

ast_node!(Root, SyntaxKind::ROOT);
ast_node!(AssignmentExpr, SyntaxKind::ASSIGNMENT_EXPR);
ast_node!(BinaryExpr, SyntaxKind::BINARY_EXPR);
ast_node!(UnaryExpr, SyntaxKind::UNARY_EXPR);
ast_node!(ParenExpr, SyntaxKind::PAREN_EXPR);
ast_node!(CallExpr, SyntaxKind::CALL_EXPR);
ast_node!(ArgList, SyntaxKind::ARG_LIST);
ast_node!(Arg, SyntaxKind::ARG);
ast_node!(IfExpr, SyntaxKind::IF_EXPR);
ast_node!(ForExpr, SyntaxKind::FOR_EXPR);
ast_node!(WhileExpr, SyntaxKind::WHILE_EXPR);
ast_node!(FunctionExpr, SyntaxKind::FUNCTION_EXPR);
ast_node!(BlockExpr, SyntaxKind::BLOCK_EXPR);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForExprParts {
    pub leading_comments: Vec<SyntaxToken<RLanguage>>,
    pub variable_elements: Vec<SyntaxElement<RLanguage>>,
    pub sequence_elements: Vec<SyntaxElement<RLanguage>>,
    pub post_clause_comments: Vec<SyntaxToken<RLanguage>>,
    pub body: Option<SyntaxElement<RLanguage>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhileExprParts {
    pub leading_comments: Vec<SyntaxToken<RLanguage>>,
    pub condition_elements: Vec<SyntaxElement<RLanguage>>,
    pub post_clause_comments: Vec<SyntaxToken<RLanguage>>,
    pub body: Option<SyntaxElement<RLanguage>>,
}

impl Root {
    pub fn expressions(&self) -> impl Iterator<Item = SyntaxNode> {
        self.syntax().children()
    }
}

impl AssignmentExpr {
    /// The assignment operator kind. Returns one of `ASSIGN_LEFT` (`<-`),
    /// `ASSIGN_RIGHT` (`->`), `SUPER_ASSIGN` (`<<-`), `SUPER_ASSIGN_RIGHT`
    /// (`->>`), `ASSIGN_EQ` (`=`), or `WALRUS` (`:=`).
    pub fn op_kind(&self) -> Option<SyntaxKind> {
        self.syntax()
            .children_with_tokens()
            .find_map(|element| match element {
                SyntaxElement::Token(token) if is_assignment_op(token.kind()) => Some(token.kind()),
                _ => None,
            })
    }

    /// The operator token, if present.
    pub fn op_token(&self) -> Option<SyntaxToken<RLanguage>> {
        self.syntax()
            .children_with_tokens()
            .find_map(|element| match element {
                SyntaxElement::Token(token) if is_assignment_op(token.kind()) => Some(token),
                _ => None,
            })
    }

    /// The element on the *target* side of the operator (the side that becomes
    /// the binding). For `<-`/`=`/`<<-`/`:=` this is the left element; for
    /// `->`/`->>` it is the right element.
    pub fn target_element(&self) -> Option<SyntaxElement<RLanguage>> {
        let elements: Vec<_> = self.syntax().children_with_tokens().collect();
        let op_idx = assignment_op_index(&elements)?;
        let kind = element_kind(&elements[op_idx])?;
        let (start, end) = if is_right_assign(kind) {
            (op_idx + 1, elements.len())
        } else {
            (0, op_idx)
        };
        elements[start..end]
            .iter()
            .find(|e| !is_trivia(e.kind()) && e.kind() != SyntaxKind::COMMENT)
            .cloned()
    }

    /// The element on the *value* side of the operator.
    pub fn value_element(&self) -> Option<SyntaxElement<RLanguage>> {
        let elements: Vec<_> = self.syntax().children_with_tokens().collect();
        let op_idx = assignment_op_index(&elements)?;
        let kind = element_kind(&elements[op_idx])?;
        let (start, end) = if is_right_assign(kind) {
            (0, op_idx)
        } else {
            (op_idx + 1, elements.len())
        };
        elements[start..end]
            .iter()
            .find(|e| !is_trivia(e.kind()) && e.kind() != SyntaxKind::COMMENT)
            .cloned()
    }

    /// If the target is a simple `IDENT` (or backtick-quoted identifier), the
    /// bound name as a `SmolStr`. Returns `None` for complex LHS like
    /// `dim(x) <- ...`, `x[1] <- ...`, `x$y <- ...`.
    pub fn target_name(&self) -> Option<SmolStr> {
        match self.target_element()? {
            SyntaxElement::Token(token) if token.kind() == SyntaxKind::IDENT => {
                Some(SmolStr::new(token.text()))
            }
            SyntaxElement::Token(token) if token.kind() == SyntaxKind::STRING => {
                strip_string_quotes(token.text()).map(SmolStr::new)
            }
            _ => None,
        }
    }

    /// The target name token, useful when the binding site's range is needed.
    pub fn target_name_token(&self) -> Option<SyntaxToken<RLanguage>> {
        match self.target_element()? {
            SyntaxElement::Token(token)
                if token.kind() == SyntaxKind::IDENT || token.kind() == SyntaxKind::STRING =>
            {
                Some(token)
            }
            _ => None,
        }
    }
}

impl FunctionExpr {
    pub fn lparen_index(&self) -> Option<usize> {
        let elements: Vec<_> = self.syntax().children_with_tokens().collect();
        elements.iter().position(|e| e.kind() == SyntaxKind::LPAREN)
    }

    pub fn rparen_index(&self) -> Option<usize> {
        let elements: Vec<_> = self.syntax().children_with_tokens().collect();
        let lparen_idx = self.lparen_index()?;
        let mut depth = 0usize;
        elements
            .iter()
            .enumerate()
            .skip(lparen_idx)
            .find_map(|(i, el)| match el.kind() {
                SyntaxKind::LPAREN => {
                    depth += 1;
                    None
                }
                SyntaxKind::RPAREN => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 { Some(i) } else { None }
                }
                _ => None,
            })
    }

    /// Iterate the parameters of the function. Each entry yields the parameter
    /// name and the range of its name token. Default-value tokens are skipped
    /// (function param defaults are raw tokens in the param list, not nodes).
    pub fn params(&self) -> Vec<Param> {
        let elements: Vec<_> = self.syntax().children_with_tokens().collect();
        let Some(lparen_idx) = self.lparen_index() else {
            return Vec::new();
        };
        let Some(rparen_idx) = self.rparen_index() else {
            return Vec::new();
        };
        let mut params = Vec::new();
        let mut depth = 0usize;
        let mut at_param_start = true;
        let mut i = lparen_idx + 1;
        while i < rparen_idx {
            let element = &elements[i];
            match element.kind() {
                SyntaxKind::LPAREN
                | SyntaxKind::LBRACK
                | SyntaxKind::LBRACK2
                | SyntaxKind::LBRACE => {
                    depth += 1;
                    at_param_start = false;
                }
                SyntaxKind::RPAREN
                | SyntaxKind::RBRACK
                | SyntaxKind::RBRACK2
                | SyntaxKind::RBRACE => {
                    depth = depth.saturating_sub(1);
                    at_param_start = false;
                }
                SyntaxKind::COMMA if depth == 0 => {
                    at_param_start = true;
                }
                SyntaxKind::IDENT if depth == 0 && at_param_start => {
                    if let SyntaxElement::Token(token) = element {
                        params.push(Param {
                            name: SmolStr::new(token.text()),
                            name_token: token.clone(),
                        });
                    }
                    at_param_start = false;
                }
                SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT => {}
                _ => {
                    at_param_start = false;
                }
            }
            i += 1;
        }
        params
    }

    /// The function body — the expression that follows the `)`. May be any
    /// expression node (a `BLOCK_EXPR` for `function(x) { ... }`, or any other
    /// expression for `function(x) x + 1`). Returns `None` if the body was
    /// missing or replaced by an error-recovery placeholder.
    pub fn body(&self) -> Option<SyntaxElement<RLanguage>> {
        let elements: Vec<_> = self.syntax().children_with_tokens().collect();
        let rparen_idx = self.rparen_index()?;
        elements[rparen_idx + 1..]
            .iter()
            .find(|e| !is_trivia(e.kind()) && e.kind() != SyntaxKind::COMMENT)
            .cloned()
    }
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: SmolStr,
    pub name_token: SyntaxToken<RLanguage>,
}

fn is_assignment_op(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::ASSIGN_LEFT
            | SyntaxKind::ASSIGN_RIGHT
            | SyntaxKind::SUPER_ASSIGN
            | SyntaxKind::SUPER_ASSIGN_RIGHT
            | SyntaxKind::ASSIGN_EQ
            | SyntaxKind::WALRUS
    )
}

fn is_right_assign(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::ASSIGN_RIGHT | SyntaxKind::SUPER_ASSIGN_RIGHT
    )
}

fn assignment_op_index(elements: &[SyntaxElement<RLanguage>]) -> Option<usize> {
    elements
        .iter()
        .position(|e| matches!(e, SyntaxElement::Token(t) if is_assignment_op(t.kind())))
}

fn element_kind(element: &SyntaxElement<RLanguage>) -> Option<SyntaxKind> {
    Some(element.kind())
}

fn strip_string_quotes(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' || first == b'\'' || first == b'`') && first == last {
            return Some(text[1..text.len() - 1].to_string());
        }
    }
    None
}

impl CallExpr {
    pub fn arg_list(&self) -> Option<ArgList> {
        support::child(self.syntax())
    }

    /// The callee name token — the `IDENT`/`STRING` immediately preceding the
    /// argument list, when the function being called is a simple name (`foo(…)`,
    /// `` `+`(…) ``). Returns `None` for a computed callee (`(g())(…)`,
    /// `x$f(…)`).
    pub fn callee_token(&self) -> Option<SyntaxToken<RLanguage>> {
        for element in self.syntax().children_with_tokens() {
            match element {
                SyntaxElement::Token(token)
                    if is_trivia(token.kind()) || token.kind() == SyntaxKind::COMMENT =>
                {
                    continue;
                }
                SyntaxElement::Token(token)
                    if matches!(token.kind(), SyntaxKind::IDENT | SyntaxKind::STRING) =>
                {
                    return Some(token);
                }
                _ => return None,
            }
        }
        None
    }
}

/// A resolved namespace access (`pkg::name` / `pkg:::name`), as extracted from a
/// [`BinaryExpr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceAccess {
    pub package: SmolStr,
    pub package_token: SyntaxToken<RLanguage>,
    /// `true` for the internal `:::` operator, `false` for `::`.
    pub internal: bool,
    pub name: SmolStr,
    pub name_token: SyntaxToken<RLanguage>,
}

impl BinaryExpr {
    /// If this binary expression is a namespace access — `pkg::name`,
    /// `pkg:::name`, or the call form `pkg::name(args)` — the resolved package
    /// and accessed name with their tokens. The package and name may be written
    /// as backtick/quoted strings, which are unquoted.
    pub fn namespace_access(&self) -> Option<NamespaceAccess> {
        let elements: Vec<_> = self.syntax().children_with_tokens().collect();
        let (op_idx, internal) = elements
            .iter()
            .enumerate()
            .find_map(|(i, e)| match e.kind() {
                SyntaxKind::COLON2 => Some((i, false)),
                SyntaxKind::COLON3 => Some((i, true)),
                _ => None,
            })?;
        // Package: the last name token before the operator.
        let package_token = elements[..op_idx].iter().rev().find_map(|e| match e {
            SyntaxElement::Token(t)
                if matches!(t.kind(), SyntaxKind::IDENT | SyntaxKind::STRING) =>
            {
                Some(t.clone())
            }
            _ => None,
        })?;
        // Name: the first non-trivia element after the operator — either a bare
        // name token or the callee of a `pkg::name(args)` call.
        let rhs = elements[op_idx + 1..]
            .iter()
            .find(|e| !is_trivia(e.kind()) && e.kind() != SyntaxKind::COMMENT)?;
        let name_token = match rhs {
            SyntaxElement::Token(t)
                if matches!(t.kind(), SyntaxKind::IDENT | SyntaxKind::STRING) =>
            {
                t.clone()
            }
            SyntaxElement::Node(n) if n.kind() == SyntaxKind::CALL_EXPR => {
                CallExpr::cast(n.clone())?.callee_token()?
            }
            _ => return None,
        };
        Some(NamespaceAccess {
            package: token_name(&package_token),
            package_token,
            internal,
            name: token_name(&name_token),
            name_token,
        })
    }
}

/// The textual name a token denotes: the raw text for an `IDENT`, or the
/// unquoted contents for a backtick/quoted `STRING`.
fn token_name(token: &SyntaxToken<RLanguage>) -> SmolStr {
    if token.kind() == SyntaxKind::STRING
        && let Some(inner) = strip_string_quotes(token.text())
    {
        return SmolStr::new(inner);
    }
    SmolStr::new(token.text())
}

impl ArgList {
    pub fn args(&self) -> impl Iterator<Item = Arg> {
        support::children(self.syntax())
    }
}

impl IfExpr {
    pub fn elements(&self) -> Vec<SyntaxElement<RLanguage>> {
        self.syntax().children_with_tokens().collect()
    }

    pub fn if_keyword(&self) -> Option<SyntaxToken<RLanguage>> {
        self.elements()
            .into_iter()
            .find_map(|element| match element {
                SyntaxElement::Token(token) if token.kind() == SyntaxKind::IF_KW => Some(token),
                _ => None,
            })
    }

    pub fn condition_elements(&self) -> Option<Vec<SyntaxElement<RLanguage>>> {
        let elements = self.elements();
        let lparen_idx = self.lparen_index()?;
        let rparen_idx = self.rparen_index()?;
        Some(elements[lparen_idx + 1..rparen_idx].to_vec())
    }

    pub fn then_elements(&self) -> Option<Vec<SyntaxElement<RLanguage>>> {
        let elements = self.elements();
        let rparen_idx = self.rparen_index()?;
        let else_idx = find_token_after_index(&elements, rparen_idx, SyntaxKind::ELSE_KW);
        let then_end = else_idx.unwrap_or(elements.len());
        Some(elements[rparen_idx + 1..then_end].to_vec())
    }

    pub fn else_keyword(&self) -> Option<SyntaxToken<RLanguage>> {
        self.elements()
            .into_iter()
            .find_map(|element| match element {
                SyntaxElement::Token(token) if token.kind() == SyntaxKind::ELSE_KW => Some(token),
                _ => None,
            })
    }

    pub fn else_elements(&self) -> Option<Vec<SyntaxElement<RLanguage>>> {
        let elements = self.elements();
        let else_idx = find_token_index(&elements, SyntaxKind::ELSE_KW)?;
        Some(elements[else_idx + 1..].to_vec())
    }

    pub fn lparen_index(&self) -> Option<usize> {
        find_token_index(&self.elements(), SyntaxKind::LPAREN)
    }

    pub fn rparen_index(&self) -> Option<usize> {
        let elements = self.elements();
        let lparen_idx = self.lparen_index()?;
        find_token_after_index(&elements, lparen_idx, SyntaxKind::RPAREN)
    }
}

impl ForExpr {
    pub fn elements(&self) -> Vec<SyntaxElement<RLanguage>> {
        self.syntax().children_with_tokens().collect()
    }

    pub fn for_keyword(&self) -> Option<SyntaxToken<RLanguage>> {
        self.elements()
            .into_iter()
            .find_map(|element| match element {
                SyntaxElement::Token(token) if token.kind() == SyntaxKind::FOR_KW => Some(token),
                _ => None,
            })
    }

    pub fn clause_bounds(&self) -> Option<(usize, usize)> {
        let elements = self.elements();
        let lparen_idx = self.lparen_index()?;

        let mut depth = 0usize;
        let rparen_idx = elements.iter().enumerate().skip(lparen_idx).find_map(
            |(idx, element)| match element {
                SyntaxElement::Token(token) if token.kind() == SyntaxKind::LPAREN => {
                    depth += 1;
                    None
                }
                SyntaxElement::Token(token) if token.kind() == SyntaxKind::RPAREN => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 { Some(idx) } else { None }
                }
                _ => None,
            },
        )?;

        Some((lparen_idx, rparen_idx))
    }

    pub fn lparen_index(&self) -> Option<usize> {
        let elements = self.elements();
        let for_idx = find_token_index(&elements, SyntaxKind::FOR_KW)?;
        find_token_after_index(&elements, for_idx, SyntaxKind::LPAREN)
    }

    pub fn leading_comments(&self) -> Option<Vec<SyntaxToken<RLanguage>>> {
        let elements = self.elements();
        let for_idx = find_token_index(&elements, SyntaxKind::FOR_KW)?;
        let (_, rparen_idx) = self.clause_bounds()?;
        Some(
            elements[for_idx + 1..rparen_idx]
                .iter()
                .filter_map(|element| match element {
                    SyntaxElement::Token(token) if token.kind() == SyntaxKind::COMMENT => {
                        Some(token.clone())
                    }
                    _ => None,
                })
                .collect(),
        )
    }

    pub fn clause_elements(&self) -> Option<Vec<SyntaxElement<RLanguage>>> {
        let elements = self.elements();
        let (lparen_idx, rparen_idx) = self.clause_bounds()?;
        Some(
            elements[lparen_idx + 1..rparen_idx]
                .iter()
                .filter(|element| {
                    !is_trivia(element.kind()) && element.kind() != SyntaxKind::COMMENT
                })
                .cloned()
                .collect(),
        )
    }

    pub fn post_clause_comments(&self) -> Option<Vec<SyntaxToken<RLanguage>>> {
        let elements = self.elements();
        let (_, rparen_idx) = self.clause_bounds()?;
        let mut comments = Vec::new();
        for element in &elements[rparen_idx + 1..] {
            if is_trivia(element.kind()) {
                continue;
            }
            if let SyntaxElement::Token(token) = element
                && token.kind() == SyntaxKind::COMMENT
            {
                comments.push(token.clone());
                continue;
            }
            break;
        }
        Some(comments)
    }

    pub fn body_element(&self) -> Option<SyntaxElement<RLanguage>> {
        let elements = self.elements();
        let (_, rparen_idx) = self.clause_bounds()?;
        for element in &elements[rparen_idx + 1..] {
            if is_trivia(element.kind()) {
                continue;
            }
            if matches!(element, SyntaxElement::Token(token) if token.kind() == SyntaxKind::COMMENT)
            {
                continue;
            }
            return Some(element.clone());
        }
        None
    }

    pub fn parts(&self) -> Option<ForExprParts> {
        self.for_keyword()?;
        self.lparen_index()?;
        self.clause_bounds()?;

        let clause_elements = self.clause_elements()?;
        let in_idx = clause_elements.iter().position(
            |el| matches!(el, SyntaxElement::Token(tok) if tok.kind() == SyntaxKind::IN_KW),
        )?;

        Some(ForExprParts {
            leading_comments: self.leading_comments()?,
            variable_elements: clause_elements[..in_idx].to_vec(),
            sequence_elements: clause_elements[in_idx + 1..].to_vec(),
            post_clause_comments: self.post_clause_comments()?,
            body: self.body_element(),
        })
    }
}

impl WhileExpr {
    pub fn elements(&self) -> Vec<SyntaxElement<RLanguage>> {
        self.syntax().children_with_tokens().collect()
    }

    pub fn while_keyword(&self) -> Option<SyntaxToken<RLanguage>> {
        self.elements()
            .into_iter()
            .find_map(|element| match element {
                SyntaxElement::Token(token) if token.kind() == SyntaxKind::WHILE_KW => Some(token),
                _ => None,
            })
    }

    pub fn clause_bounds(&self) -> Option<(usize, usize)> {
        let elements = self.elements();
        let lparen_idx = self.lparen_index()?;

        let mut depth = 0usize;
        let rparen_idx = elements.iter().enumerate().skip(lparen_idx).find_map(
            |(idx, element)| match element {
                SyntaxElement::Token(token) if token.kind() == SyntaxKind::LPAREN => {
                    depth += 1;
                    None
                }
                SyntaxElement::Token(token) if token.kind() == SyntaxKind::RPAREN => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 { Some(idx) } else { None }
                }
                _ => None,
            },
        )?;

        Some((lparen_idx, rparen_idx))
    }

    pub fn lparen_index(&self) -> Option<usize> {
        let elements = self.elements();
        let while_idx = find_token_index(&elements, SyntaxKind::WHILE_KW)?;
        find_token_after_index(&elements, while_idx, SyntaxKind::LPAREN)
    }

    pub fn leading_comments(&self) -> Option<Vec<SyntaxToken<RLanguage>>> {
        let elements = self.elements();
        let while_idx = find_token_index(&elements, SyntaxKind::WHILE_KW)?;
        let (_, rparen_idx) = self.clause_bounds()?;
        Some(
            elements[while_idx + 1..rparen_idx]
                .iter()
                .filter_map(|element| match element {
                    SyntaxElement::Token(token) if token.kind() == SyntaxKind::COMMENT => {
                        Some(token.clone())
                    }
                    _ => None,
                })
                .collect(),
        )
    }

    pub fn condition_elements(&self) -> Option<Vec<SyntaxElement<RLanguage>>> {
        let elements = self.elements();
        let (lparen_idx, rparen_idx) = self.clause_bounds()?;
        Some(
            elements[lparen_idx + 1..rparen_idx]
                .iter()
                .filter(|element| {
                    !is_trivia(element.kind()) && element.kind() != SyntaxKind::COMMENT
                })
                .cloned()
                .collect(),
        )
    }

    pub fn post_clause_comments(&self) -> Option<Vec<SyntaxToken<RLanguage>>> {
        let elements = self.elements();
        let (_, rparen_idx) = self.clause_bounds()?;
        let mut comments = Vec::new();
        for element in &elements[rparen_idx + 1..] {
            if is_trivia(element.kind()) {
                continue;
            }
            if let SyntaxElement::Token(token) = element
                && token.kind() == SyntaxKind::COMMENT
            {
                comments.push(token.clone());
                continue;
            }
            break;
        }
        Some(comments)
    }

    pub fn body_element(&self) -> Option<SyntaxElement<RLanguage>> {
        let elements = self.elements();
        let (_, rparen_idx) = self.clause_bounds()?;
        for element in &elements[rparen_idx + 1..] {
            if is_trivia(element.kind()) {
                continue;
            }
            if matches!(element, SyntaxElement::Token(token) if token.kind() == SyntaxKind::COMMENT)
            {
                continue;
            }
            return Some(element.clone());
        }
        None
    }

    pub fn parts(&self) -> Option<WhileExprParts> {
        self.while_keyword()?;
        self.lparen_index()?;
        self.clause_bounds()?;

        Some(WhileExprParts {
            leading_comments: self.leading_comments()?,
            condition_elements: self.condition_elements()?,
            post_clause_comments: self.post_clause_comments()?,
            body: self.body_element(),
        })
    }
}

fn find_token_index(elements: &[SyntaxElement<RLanguage>], kind: SyntaxKind) -> Option<usize> {
    elements
        .iter()
        .position(|element| matches!(element, SyntaxElement::Token(token) if token.kind() == kind))
}

fn find_token_after_index(
    elements: &[SyntaxElement<RLanguage>],
    start_idx: usize,
    kind: SyntaxKind,
) -> Option<usize> {
    elements
        .iter()
        .enumerate()
        .skip(start_idx + 1)
        .find_map(|(idx, element)| match element {
            SyntaxElement::Token(token) if token.kind() == kind => Some(idx),
            _ => None,
        })
}

fn is_trivia(kind: SyntaxKind) -> bool {
    matches!(kind, SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE)
}
