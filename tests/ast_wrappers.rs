use arity::ast::{
    Arg, ArgList, AssignmentExpr, AstNode, AstToken, BinaryExpr, BlockExpr, CallExpr, Comment,
    Expr, FloatLit, ForExpr, FunctionExpr, HasArgList, Ident, IfExpr, IntLit, ParenExpr, RConstant,
    RepeatExpr, RoxygenBlock, StringLit, Subset2Expr, SubsetExpr, UnaryExpr,
};
use arity::parser::parse;
use arity::syntax::{SyntaxElement, SyntaxKind};

fn first_node<N: AstNode<Language = arity::syntax::RLanguage>>(src: &str) -> N {
    let parsed = parse(src);
    assert!(
        parsed.diagnostics.is_empty(),
        "parse: {:?}",
        parsed.diagnostics
    );
    parsed.cst.descendants().find_map(N::cast).expect("a node")
}

fn element_text(el: &SyntaxElement) -> String {
    match (el.as_token(), el.as_node()) {
        (Some(t), _) => t.text().to_string(),
        (_, Some(n)) => n.text().to_string(),
        _ => unreachable!(),
    }
}

fn first_binary(src: &str) -> BinaryExpr {
    let parsed = parse(src);
    assert!(
        parsed.diagnostics.is_empty(),
        "parse: {:?}",
        parsed.diagnostics
    );
    parsed
        .cst
        .descendants()
        .find_map(BinaryExpr::cast)
        .expect("a binary expression")
}

#[test]
fn namespace_access_plain_reference() {
    let na = first_binary("dplyr::filter")
        .namespace_access()
        .expect("namespace access");
    assert_eq!(na.package, "dplyr");
    assert_eq!(na.name, "filter");
    assert!(!na.internal);
    assert_eq!(na.name_token.text(), "filter");
    assert_eq!(na.package_token.text(), "dplyr");
}

#[test]
fn namespace_access_call_form_uses_callee_name() {
    // `pkg::name(args)`: the name is the callee of the inner call, not an arg.
    let na = first_binary("dplyr::filter(x, y)")
        .namespace_access()
        .expect("namespace access");
    assert_eq!(na.package, "dplyr");
    assert_eq!(na.name, "filter");
    assert_eq!(na.name_token.kind(), SyntaxKind::IDENT);
}

#[test]
fn namespace_access_internal_operator() {
    let na = first_binary("rlang:::abort")
        .namespace_access()
        .expect("namespace access");
    assert!(na.internal);
    assert_eq!(na.package, "rlang");
    assert_eq!(na.name, "abort");
}

#[test]
fn non_namespace_binary_has_no_access() {
    assert!(first_binary("a + b").namespace_access().is_none());
}

#[test]
fn callee_token_resolves_simple_call() {
    let parsed = parse("foo(1, 2)");
    let call = parsed
        .cst
        .descendants()
        .find_map(CallExpr::cast)
        .expect("a call");
    assert_eq!(call.callee_token().expect("callee").text(), "foo");
}

#[test]
fn callee_token_none_for_computed_callee() {
    // `(g)(x)`: the callee is a parenthesized expression, not a name.
    let parsed = parse("(g)(x)");
    let call = parsed
        .cst
        .descendants()
        .find_map(CallExpr::cast)
        .expect("a call");
    assert!(call.callee_token().is_none());
}

#[test]
fn casts_core_expression_wrappers() {
    let parsed = parse(
        "x <- function(a, b) { a + b }\nz <- fn(1, 2)\nif (x) { y <- 1 + 2 }\nfor (i in 1:5) i\n",
    );
    assert!(
        parsed.diagnostics.is_empty(),
        "fixture should parse cleanly: {:?}",
        parsed.diagnostics
    );

    let mut saw_assignment = false;
    let mut saw_call = false;
    let mut saw_arg_list = false;
    let mut saw_if = false;
    let mut saw_block = false;
    let mut saw_binary = false;
    let mut saw_for = false;
    let mut saw_function = false;

    for node in parsed.cst.descendants() {
        if AssignmentExpr::cast(node.clone()).is_some() {
            saw_assignment = true;
        }
        if CallExpr::cast(node.clone()).is_some() {
            saw_call = true;
        }
        if ArgList::cast(node.clone()).is_some() {
            saw_arg_list = true;
        }
        if IfExpr::cast(node.clone()).is_some() {
            saw_if = true;
        }
        if BlockExpr::cast(node.clone()).is_some() {
            saw_block = true;
        }
        if BinaryExpr::cast(node.clone()).is_some() {
            saw_binary = true;
        }
        if ForExpr::cast(node.clone()).is_some() {
            saw_for = true;
        }
        if FunctionExpr::cast(node).is_some() {
            saw_function = true;
        }
    }

    assert!(saw_assignment);
    assert!(saw_call);
    assert!(saw_arg_list);
    assert!(saw_if);
    assert!(saw_block);
    assert!(saw_binary);
    assert!(saw_for);
    assert!(saw_function);
}

#[test]
fn if_expr_accessors_expose_structural_parts() {
    let parsed = parse("if (x) y else z\n");
    assert!(parsed.diagnostics.is_empty());

    let if_expr = parsed
        .cst
        .descendants()
        .find_map(IfExpr::cast)
        .expect("expected if expression");

    assert!(if_expr.if_keyword().is_some());
    assert!(if_expr.else_keyword().is_some());
    assert!(if_expr.lparen_index().is_some());
    assert!(if_expr.rparen_index().is_some());

    let condition = if_expr
        .condition_elements()
        .expect("expected condition elements");
    assert!(
        condition
            .iter()
            .any(|element| element.kind() == SyntaxKind::IDENT)
    );

    let then_elements = if_expr.then_elements().expect("expected then branch");
    assert!(
        then_elements
            .iter()
            .any(|element| element.kind() == SyntaxKind::IDENT)
    );

    let else_elements = if_expr.else_elements().expect("expected else branch");
    assert!(
        else_elements
            .iter()
            .any(|element| element.kind() == SyntaxKind::IDENT)
    );
}

#[test]
fn for_expr_accessors_expose_clause_and_body() {
    let parsed = parse("for (\n# lead\ni in xs\n) i\n");
    assert!(parsed.diagnostics.is_empty());

    let for_expr = parsed
        .cst
        .descendants()
        .find_map(ForExpr::cast)
        .expect("expected for expression");

    assert!(for_expr.for_keyword().is_some());
    assert!(for_expr.lparen_index().is_some());
    assert!(for_expr.clause_bounds().is_some());

    let leading_comments = for_expr
        .leading_comments()
        .expect("expected leading comments");
    assert_eq!(leading_comments.len(), 1);
    assert_eq!(leading_comments[0].kind(), SyntaxKind::COMMENT);

    let clause_elements = for_expr
        .clause_elements()
        .expect("expected clause elements");
    assert_eq!(clause_elements.len(), 3);
    assert_eq!(clause_elements[0].kind(), SyntaxKind::IDENT);
    assert_eq!(clause_elements[1].kind(), SyntaxKind::IN_KW);
    assert_eq!(clause_elements[2].kind(), SyntaxKind::IDENT);

    let post_comments = for_expr
        .post_clause_comments()
        .expect("expected post-clause comments");
    assert!(post_comments.is_empty());

    let body = for_expr.body_element().expect("expected body");
    assert_eq!(body.kind(), SyntaxKind::IDENT);
}

#[test]
fn for_expr_accessors_capture_post_clause_comments() {
    let parsed = parse("for (i in xs) # post\n");
    assert!(parsed.diagnostics.is_empty());

    let for_expr = parsed
        .cst
        .descendants()
        .find_map(ForExpr::cast)
        .expect("expected for expression");
    let post_comments = for_expr.post_clause_comments().expect("expected comments");
    assert_eq!(post_comments.len(), 1);
    assert_eq!(post_comments[0].kind(), SyntaxKind::COMMENT);
    assert!(for_expr.body_element().is_none());
}

fn first_roxygen_block(src: &str) -> RoxygenBlock {
    let parsed = parse(src);
    assert!(
        parsed.diagnostics.is_empty(),
        "parse: {:?}",
        parsed.diagnostics
    );
    parsed
        .cst
        .descendants()
        .find_map(RoxygenBlock::cast)
        .expect("a roxygen block")
}

#[test]
fn roxygen_block_sections_classify_intro_and_tags() {
    let block = first_roxygen_block(
        "#' Title\n#'\n#' @param x A number.\n#' @examples\nf <- function(x) x\n",
    );
    // The block owns logical sections: the intro (untagged prose), then one
    // section per `@tag`.
    let sections: Vec<_> = block.sections().collect();
    assert_eq!(sections.len(), 3);

    // Intro section: no tag heading, a single prose paragraph (the blank `#'`
    // line is a separator, not its own paragraph).
    assert!(sections[0].tag().is_none());
    assert_eq!(sections[0].paragraphs().count(), 1);

    // `@param x ...`: tag with name + arg + trailing text.
    let param = sections[1].tag().expect("param tag");
    assert_eq!(param.name().as_deref(), Some("param"));
    assert_eq!(param.arg().unwrap().text(), "x");
    assert_eq!(param.text().unwrap().text(), "A number.");
    assert!(!param.is_examples());

    // `@examples`: an examples tag with no arg/text.
    let examples = sections[2].tag().expect("examples tag");
    assert!(examples.is_examples());
    assert!(examples.arg().is_none());
}

#[test]
fn roxygen_block_tag_lookup() {
    let block = first_roxygen_block(
        "#' Title\n#'\n#' @param x A number.\n#' @export\nf <- function(x) x\n",
    );
    let names: Vec<_> = block.tags().filter_map(|t| t.name()).collect();
    assert_eq!(names, ["param", "export"]);
    assert!(block.has_tag("export"));
    assert!(!block.has_tag("return"));

    // The intro is the untagged leading section.
    let intro = block.intro().expect("intro section");
    assert!(intro.tag().is_none());
    assert_eq!(intro.paragraphs().count(), 1);
}

#[test]
fn roxygen_block_without_intro() {
    let block = first_roxygen_block("#' @export\nf <- function() 1\n");
    assert!(block.intro().is_none());
    assert!(block.has_tag("export"));
}

#[test]
fn roxygen_tag_arg_names_splits_comma_lists() {
    let src = "#' @param a,b Both arguments.\nf <- function(a, b) a\n";
    let block = first_roxygen_block(src);
    let tag = block.tags().next().expect("param tag");
    let names = tag.arg_names();
    assert_eq!(names.len(), 2);
    // Each name carries the sub-range of its own text inside the arg token.
    assert_eq!(names[0].0, "a");
    assert_eq!(&src[names[0].1], "a");
    assert_eq!(names[1].0, "b");
    assert_eq!(&src[names[1].1], "b");
}

#[test]
fn roxygen_tag_arg_names_single_and_missing() {
    let block = first_roxygen_block("#' @param x A number.\n#' @param\nf <- function(x) x\n");
    let tags: Vec<_> = block.tags().collect();
    let names = tags[0].arg_names();
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].0, "x");
    // A bare `@param` has no arg token, hence no names.
    assert!(tags[1].arg_names().is_empty());
}

#[test]
fn roxygen_section_has_prose() {
    let block = first_roxygen_block(
        "#' @param x A number.\n#' @param y\n#'   Continuation.\n#' @param z\n#' @export\nf <- function(x, y, z) x\n",
    );
    let sections: Vec<_> = block.sections().collect();
    // `@param x A number.` — description folded onto the tag line.
    assert!(sections[0].has_prose());
    // `@param y` + continuation paragraph.
    assert!(sections[1].has_prose());
    // `@param z` with no description at all.
    assert!(!sections[2].has_prose());
    // `@export` carries no prose.
    assert!(!sections[3].has_prose());
}

// --- Phase 1: token wrappers ------------------------------------------------

fn first_ident(src: &str) -> Ident {
    parse(src)
        .cst
        .descendants_with_tokens()
        .find_map(|e| e.into_token().and_then(Ident::cast))
        .expect("an ident token")
}

#[test]
fn ident_constant_classifies_special_symbols() {
    assert_eq!(first_ident("TRUE").constant(), Some(RConstant::True));
    assert_eq!(first_ident("FALSE").constant(), Some(RConstant::False));
    assert_eq!(first_ident("NA").constant(), Some(RConstant::Na));
    assert_eq!(
        first_ident("NA_integer_").constant(),
        Some(RConstant::NaInteger)
    );
    assert_eq!(first_ident("NULL").constant(), Some(RConstant::Null));
    assert_eq!(first_ident("NaN").constant(), Some(RConstant::NaN));
    assert_eq!(first_ident("Inf").constant(), Some(RConstant::Inf));
    assert_eq!(first_ident("T").constant(), Some(RConstant::TSymbol));
    assert_eq!(first_ident("F").constant(), Some(RConstant::FSymbol));
    assert_eq!(first_ident("x").constant(), None);
}

#[test]
fn ident_boolean_and_na_helpers() {
    assert!(first_ident("TRUE").is_true());
    assert!(first_ident("FALSE").is_false());
    assert!(first_ident("NA").is_na());
    assert!(first_ident("NA_character_").is_na());
    assert!(first_ident("NULL").is_null());
    assert!(first_ident("NaN").is_nan());
    assert!(first_ident("T").is_bool_symbol());
    assert!(first_ident("F").is_bool_symbol());
    assert!(!first_ident("x").is_true());
}

#[test]
fn ident_dots_and_reserved_constant() {
    assert!(first_ident("...").is_dots());
    assert!(first_ident("..1").is_dots());
    assert!(!first_ident("x").is_dots());
    // Reserved constants exclude the rebindable T / F.
    assert!(first_ident("TRUE").is_reserved_constant());
    assert!(first_ident("NA").is_reserved_constant());
    assert!(first_ident("Inf").is_reserved_constant());
    assert!(!first_ident("T").is_reserved_constant());
    assert!(!first_ident("F").is_reserved_constant());
    assert!(!first_ident("x").is_reserved_constant());
}

#[test]
fn string_lit_quote_inner_and_unquote() {
    let call = first_node::<CallExpr>("f(\"^abc\")");
    let tok = call.arg_list().unwrap().args().next().unwrap();
    let value = tok.value().unwrap().into_token().unwrap();
    let s = StringLit::cast(value).unwrap();
    assert_eq!(s.quote(), Some('"'));
    assert_eq!(s.inner(), Some("^abc"));
    assert_eq!(s.unquote(), Some("^abc"));
    assert!(!s.is_backtick());
}

#[test]
fn string_lit_single_quote() {
    let single = first_node::<CallExpr>("f('a.b')");
    let v = single
        .arg_list()
        .unwrap()
        .args()
        .next()
        .unwrap()
        .value()
        .unwrap()
        .into_token()
        .unwrap();
    let s = StringLit::cast(v).unwrap();
    assert_eq!(s.quote(), Some('\''));
    assert_eq!(s.inner(), Some("a.b"));
    assert_eq!(s.unquote(), Some("a.b"));
    // A backtick name lexes as IDENT, not STRING, so it never casts to StringLit.
    let bt = first_node::<AssignmentExpr>("`my var` <- 1");
    let tok = bt.target_name_token().unwrap();
    assert_eq!(tok.kind(), SyntaxKind::IDENT);
    assert!(StringLit::cast(tok).is_none());
}

#[test]
fn int_float_comment_tokens() {
    assert_eq!(
        parse("1L")
            .cst
            .descendants_with_tokens()
            .find_map(|e| e.into_token().and_then(IntLit::cast))
            .unwrap()
            .text(),
        "1L"
    );
    assert_eq!(
        parse("1.5")
            .cst
            .descendants_with_tokens()
            .find_map(|e| e.into_token().and_then(FloatLit::cast))
            .unwrap()
            .text(),
        "1.5"
    );
    assert_eq!(
        parse("# hi\n")
            .cst
            .descendants_with_tokens()
            .find_map(|e| e.into_token().and_then(Comment::cast))
            .unwrap()
            .text(),
        "# hi"
    );
}

// --- Phase 1: node accessors ------------------------------------------------

#[test]
fn binary_expr_parts() {
    let b = first_binary("x == TRUE");
    let (lhs, op, rhs) = b.parts().unwrap();
    assert_eq!(element_text(&lhs), "x");
    assert_eq!(op.kind(), SyntaxKind::EQUAL2);
    assert_eq!(element_text(&rhs), "TRUE");
    assert_eq!(element_text(&b.lhs().unwrap()), "x");
    assert_eq!(b.op_kind(), Some(SyntaxKind::EQUAL2));
    assert_eq!(element_text(&b.rhs().unwrap()), "TRUE");
}

#[test]
fn binary_expr_compound_operand_is_node() {
    let b = first_binary("a + b == c");
    // The top operator is `==`; its lhs is the compound `a + b` node.
    assert_eq!(b.op_kind(), Some(SyntaxKind::EQUAL2));
    let lhs = b.lhs().unwrap();
    assert!(lhs.as_node().is_some());
    assert_eq!(element_text(&lhs), "a + b");
}

#[test]
fn unary_expr_accessors() {
    let u = first_node::<UnaryExpr>("!x");
    assert_eq!(u.op_kind(), Some(SyntaxKind::BANG));
    assert_eq!(element_text(&u.operand().unwrap()), "x");
    let u = first_node::<UnaryExpr>("-y");
    assert_eq!(u.op_kind(), Some(SyntaxKind::MINUS));
    assert_eq!(element_text(&u.operand().unwrap()), "y");
}

#[test]
fn paren_expr_inner() {
    let p = first_node::<ParenExpr>("(a + b)");
    assert!(p.lparen().is_some());
    assert!(p.rparen().is_some());
    assert_eq!(element_text(&p.inner().unwrap()), "a + b");
}

#[test]
fn block_expr_statements() {
    let b = first_node::<BlockExpr>("{ a; b\n c }");
    assert!(b.lbrace().is_some());
    assert!(b.rbrace().is_some());
    let stmts: Vec<String> = b.statements().map(|e| element_text(&e)).collect();
    assert_eq!(stmts, vec!["a", "b", "c"]);
}

#[test]
fn arg_name_and_value() {
    let call = first_node::<CallExpr>("f(1, b = 2)");
    let args: Vec<Arg> = call.arg_list().unwrap().args().collect();
    // Positional first arg.
    assert!(!args[0].is_named());
    assert_eq!(args[0].name(), None);
    assert_eq!(element_text(&args[0].value().unwrap()), "1");
    // Named second arg.
    assert!(args[1].is_named());
    assert_eq!(args[1].name().as_deref(), Some("b"));
    assert!(args[1].eq_token().is_some());
    assert_eq!(element_text(&args[1].value().unwrap()), "2");
}

#[test]
fn repeat_expr_body() {
    let r = first_node::<RepeatExpr>("repeat break");
    assert!(r.repeat_keyword().is_some());
    assert_eq!(element_text(&r.body().unwrap()), "break");
}

#[test]
fn subset_expr_accessors() {
    let s = first_node::<SubsetExpr>("x[1, 2]");
    assert_eq!(element_text(&s.base().unwrap()), "x");
    assert_eq!(s.open_bracket().unwrap().kind(), SyntaxKind::LBRACK);
    assert_eq!(s.close_bracket().unwrap().kind(), SyntaxKind::RBRACK);
    let args: Vec<String> = s
        .args()
        .map(|a| element_text(&a.value().unwrap()))
        .collect();
    assert_eq!(args, vec!["1", "2"]);
}

#[test]
fn subset2_expr_accessors() {
    let s = first_node::<Subset2Expr>("x[[2]]");
    assert_eq!(element_text(&s.base().unwrap()), "x");
    assert_eq!(s.open_bracket().unwrap().kind(), SyntaxKind::LBRACK2);
    assert_eq!(s.close_bracket().unwrap().kind(), SyntaxKind::RBRACK2);
    let args: Vec<String> = s
        .args()
        .map(|a| element_text(&a.value().unwrap()))
        .collect();
    assert_eq!(args, vec!["2"]);
}

#[test]
fn call_expr_callee_name_and_base() {
    let call = first_node::<CallExpr>("foo(1)");
    assert_eq!(call.callee_name().as_deref(), Some("foo"));
    assert_eq!(element_text(&call.base().unwrap()), "foo");
    // Computed callee: no simple name, but base is the node.
    let call = first_node::<CallExpr>("(g())(1)");
    assert_eq!(call.callee_name(), None);
    assert_eq!(element_text(&call.base().unwrap()), "(g())");
}

// --- Phase 2: Expr enum + HasArgList ----------------------------------------

fn first_binary_rhs_lhs(src: &str) -> (SyntaxElement, SyntaxElement) {
    let b = first_binary(src);
    let (lhs, _, rhs) = b.parts().unwrap();
    (lhs, rhs)
}

#[test]
fn expr_cast_classifies_token_atoms() {
    let (lhs, rhs) = first_binary_rhs_lhs("x == TRUE");
    assert!(matches!(Expr::cast(lhs), Some(Expr::Name(_))));
    // `TRUE` is a Name atom; distinguish the constant via Ident::constant.
    match Expr::cast(rhs) {
        Some(Expr::Name(id)) => assert_eq!(id.constant(), Some(RConstant::True)),
        other => panic!("expected Name, got {other:?}"),
    }
    let (int, _) = first_binary_rhs_lhs("1 + x");
    assert!(matches!(Expr::cast(int), Some(Expr::IntLiteral(_))));
    let (flt, _) = first_binary_rhs_lhs("1.5 + x");
    assert!(matches!(Expr::cast(flt), Some(Expr::FloatLiteral(_))));
}

#[test]
fn expr_cast_classifies_compound_nodes() {
    let node = |src: &str| -> SyntaxElement {
        parse(src)
            .cst
            .first_child()
            .expect("a top-level node")
            .into()
    };
    assert!(matches!(
        Expr::cast(node("x <- 1")),
        Some(Expr::Assignment(_))
    ));
    assert!(matches!(Expr::cast(node("a + b")), Some(Expr::Binary(_))));
    assert!(matches!(Expr::cast(node("!x")), Some(Expr::Unary(_))));
    assert!(matches!(Expr::cast(node("(x)")), Some(Expr::Paren(_))));
    assert!(matches!(Expr::cast(node("f(1)")), Some(Expr::Call(_))));
    assert!(matches!(Expr::cast(node("x[1]")), Some(Expr::Subset(_))));
    assert!(matches!(Expr::cast(node("x[[1]]")), Some(Expr::Subset2(_))));
    assert!(matches!(Expr::cast(node("if (x) y")), Some(Expr::If(_))));
    assert!(matches!(
        Expr::cast(node("for (i in x) i")),
        Some(Expr::For(_))
    ));
    assert!(matches!(
        Expr::cast(node("while (x) y")),
        Some(Expr::While(_))
    ));
    assert!(matches!(
        Expr::cast(node("repeat break")),
        Some(Expr::Repeat(_))
    ));
    assert!(matches!(
        Expr::cast(node("function(x) x")),
        Some(Expr::Function(_))
    ));
    assert!(matches!(Expr::cast(node("{ x }")), Some(Expr::Block(_))));
}

#[test]
fn expr_cast_rejects_non_expressions() {
    // An operator token is not an expression.
    let b = first_binary("a + b");
    let op = b.op().unwrap();
    assert!(Expr::cast(op.into()).is_none());
    // An ARG_LIST node is not an expression.
    let call = first_node::<CallExpr>("f(1)");
    let arg_list = call.arg_list().unwrap();
    assert!(Expr::cast_node(arg_list.syntax().clone()).is_none());
}

#[test]
fn expr_syntax_and_text_range_round_trip() {
    let (lhs, _) = first_binary_rhs_lhs("foo + 1");
    let expr = Expr::cast(lhs.clone()).unwrap();
    assert_eq!(expr.syntax(), lhs);
    assert_eq!(expr.text_range(), lhs.text_range());
}

#[test]
fn expr_is_atom_guards_negation() {
    // `x` is a primary atom.
    let (lhs, _) = first_binary_rhs_lhs("x == FALSE");
    assert!(Expr::cast(lhs).unwrap().is_atom());
    // `a > b` is a binary expr, not a primary — negation would misparse.
    let (lhs, _) = first_binary_rhs_lhs("a > b == FALSE");
    assert!(!Expr::cast(lhs).unwrap().is_atom());
}

#[test]
fn has_arg_list_positional_and_named() {
    let call = first_node::<CallExpr>("f(1, b = 2, 3)");
    assert_eq!(element_text(&call.nth_positional(0).unwrap()), "1");
    // `b = 2` is named, so positional indexing skips it.
    assert_eq!(element_text(&call.nth_positional(1).unwrap()), "3");
    assert_eq!(element_text(&call.named_arg("b").unwrap()), "2");
    assert!(call.named_arg("z").is_none());
}

#[test]
fn has_arg_list_works_for_subscripts() {
    let subset = first_node::<SubsetExpr>("x[i, j = 2]");
    assert_eq!(element_text(&subset.nth_positional(0).unwrap()), "i");
    assert_eq!(element_text(&subset.named_arg("j").unwrap()), "2");
    let subset2 = first_node::<Subset2Expr>("x[[k]]");
    assert_eq!(element_text(&subset2.nth_positional(0).unwrap()), "k");
}
