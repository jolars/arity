use std::{fs, path::Path};

use insta::assert_snapshot;

use arity::parser::{parse, reconstruct};

#[test]
fn parser_fixtures_snapshots_and_losslessness() {
    for name in fixture_names() {
        let input = fixture_input(name);
        let output = parse(&input);
        let tree = format!("{:#?}", output.cst);
        assert_snapshot!(format!("{name}_cst"), tree);
        assert_snapshot!(
            format!("{name}_diagnostics"),
            format!("{:#?}", output.diagnostics)
        );

        if requires_lossless_round_trip(name) {
            let reconstructed = reconstruct(&input);
            assert_eq!(
                reconstructed, input,
                "lossless round-trip failed for {name}"
            );
        } else {
            assert!(
                !output.diagnostics.is_empty(),
                "expected diagnostics for non-lossless fixture {name}"
            );
        }
    }
}

fn fixture_input(name: &str) -> String {
    let path = Path::new("tests")
        .join("fixtures")
        .join("parser")
        .join(name)
        .join("input.R");
    fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("failed to read fixture {}: {err}", path.display());
    })
}

fn fixture_names() -> &'static [&'static str] {
    &[
        "assignment_simple",
        "assignment_float",
        "numeric_dot_leading",
        "dotdotdot_names",
        "binary_comment_after_operator",
        "assignment_string",
        "assignment_eq",
        "assignment_left2",
        "assignment_right",
        "assignment_right2",
        "comment_only",
        "user_operator_tokens",
        "double_brackets_tokens",
        "assignment_missing_rhs",
        "assignment_missing_rhs_eq",
        "expr_precedence",
        "expr_right_assoc_power",
        "expr_power_alias",
        "expr_paren_precedence",
        "assignment_expr_rhs",
        "assignment_chain_right_assoc",
        "pipe_simple",
        "pipe_precedence",
        "expr_logical_relational",
        "expr_additive_multiplicative_colon",
        "expr_tilde_userop",
        "expr_tilde_unary_and_binary",
        "expr_help_operator",
        "expr_walrus",
        "expr_extract_namespace",
        "expr_newline_binary_break",
        "expr_separators_tokens",
        "expr_unary",
        "expr_not_precedence",
        "expr_dotted_ident",
        "ident_backtick",
        "ident_bare_dot",
        "expr_missing_rhs",
        "expr_unexpected_prefix_op",
        "if_simple",
        "if_else_blocks",
        "if_missing_condition_parens",
        "if_missing_condition_expr",
        "if_missing_rparen",
        "if_missing_then_expr",
        "if_missing_else_expr",
        "if_dangling_else",
        "if_comment_in_condition",
        "for_simple",
        "for_newline_body",
        "for_missing_in",
        "for_missing_rparen",
        "while_simple",
        "while_newline_body",
        "while_missing_condition",
        "while_missing_rparen",
        "function_simple",
        "function_param_default_expr",
        "function_newline_body",
        "function_missing_body",
        "function_missing_rparen_body",
        "unclosed_block",
        "call_simple",
        "call_lambda_and_dot_named",
        "call_named_args",
        "call_named_args_without_rhs",
        "call_mixed_args",
        "stmt_semicolon_separator",
        "subset_simple",
        "subset2_simple",
        "subset_named_args",
        "subset_assignment",
        "subset_missing_close",
        "subset_nested_close",
        "air_ok_binary_expressions",
        "air_ok_braced_expressions",
        "air_ok_calls",
        "air_ok_comments",
        "air_ok_if_statement",
        "air_ok_unary_expressions",
        "air_ok_subset",
        "air_ok_subset2",
        "air_ok_extract_expression",
        "air_ok_namespace_expression",
        "air_ok_function_definition",
        "air_ok_for_statement",
        "air_ok_while_statement",
        "air_ok_value_double_value",
        "air_ok_value_integer_value",
        "air_ok_value_string_value",
        "air_ok_crlf_multiline_string_value",
        "air_ok_keyword",
        "air_ok_repeat_statement",
        "air_ok_dots",
        "air_ok_dot_dot_i",
        "air_ok_value_complex_value",
        "air_error_call_side_by_side_arguments",
        "air_error_parenthesized_expression_empty",
        "air_error_parenthesized_expression_multiple",
        "air_error_namespace_call_lhs_double_colon",
        "air_error_namespace_call_lhs_triple_colon",
        "air_error_namespace_chained_double_colon",
        "air_error_namespace_chained_triple_colon",
        "air_undefined_extract_expression_error",
        "air_undefined_namespace_expression_error",
        "air_ok_parenthesized_expression",
        "air_ok_semicolons_semicolon_end_of_file_01",
        "air_ok_semicolons_semicolon_end_of_file_02",
        "air_ok_semicolons_semicolon_end_of_file_03",
        "air_ok_semicolons_semicolon_start_of_file_01",
        "air_ok_semicolons_semicolon_start_of_file_02",
        "air_ok_semicolons_semicolons",
        "roxygen_simple",
        "roxygen_block",
        "roxygen_blank_line",
        "roxygen_tag_param",
        "roxygen_tag_examples",
        "roxygen_multi_hash",
        "roxygen_not_roxygen",
        "roxygen_shebang",
        "roxygen_mixed_run",
        "roxygen_loose_in_call",
        "roxygen_crlf",
        "roxygen_eof_no_newline",
        "roxygen_inline_code",
        "roxygen_rd_link",
        "roxygen_rd_link_pkg",
        "roxygen_rd_code",
        "roxygen_md_link",
        "roxygen_md_autolink",
        "roxygen_mixed_inline",
        "roxygen_nested_braces",
        "roxygen_rd_macro_nested",
        "roxygen_block_macro",
        "roxygen_describe_item",
        "roxygen_tabular",
        "roxygen_md_inline",
        "roxygen_unterminated_code",
        "roxygen_unbalanced_macro",
        "roxygen_backtick_in_macro",
    ]
}

fn requires_lossless_round_trip(name: &str) -> bool {
    !name.starts_with("air_error_")
}
