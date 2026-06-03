use ravel::ast::{
    ArgList, AssignmentExpr, AstNode, BinaryExpr, BlockExpr, CallExpr, ForExpr, FunctionExpr,
    IfExpr,
};
use ravel::parser::parse;
use ravel::syntax::SyntaxKind;

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
