use arity_formatter::formatter::{FormatStyle, format, format_verified, format_with_style};
use arity_formatter::parser::{parse, reconstruct};
use insta::assert_snapshot;
use std::{fs, path::Path};

fn fixture_text(name: &str, file: &str) -> String {
    let path = Path::new("tests")
        .join("fixtures")
        .join("formatter")
        .join(name)
        .join(file);
    fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("failed to read fixture {}: {err}", path.display());
    })
}

fn fixture_names() -> &'static [&'static str] {
    &[
        "air_binary_expression_subset",
        "air_binary_expression_sticky_subset",
        "air_braced_expressions",
        "air_call",
        "air_comment",
        "air_dot_dot_i",
        "air_for_statement",
        "air_function_definition",
        "air_keyword",
        "air_parenthesized_expression",
        "air_pipelines",
        "air_program",
        "air_repeat_statement",
        "air_smoke",
        "air_subset2",
        "air_test_that",
        "air_value_double_value",
        "air_value_integer_value",
        "air_value_string_value",
        "air_while_statement",
        "assignment_precedence",
        "assignment_walrus",
        "comment_blank_line_gap_preserved",
        "comment_trailing_alignment",
        "binary_comment_after_operator",
        "binary_multi_comment_after_operator",
        "binary_comment_before_operator",
        "binary_chain_comment_before_operator",
        "binary_trailing_comment_stays_inline",
        "binary_chain_break",
        "binary_paren_operand_indent",
        "directive_both_prefix",
        "directive_misplaced_is_inert",
        "directive_region",
        "directive_region_unclosed",
        "directive_skip_control_flow_body",
        "directive_skip_file",
        "directive_skip_in_block",
        "directive_skip_statement",
        "dotdotdot_length_call",
        "assignment_rhs_leading_comment",
        "assignment_rhs_roxygen_comment",
        "block_body_trailing_roxygen",
        "assignment_rhs_multi_comment",
        "assignment_rhs_breaks_before_lhs_subset",
        "assignment_rhs_breaks_before_lhs_subset_bare_body",
        "assignment_rhs_breaks_before_lhs_subset_in_function",
        "assignment_subset_lhs_with_if_rhs",
        "assignment_rhs_if_else_comment",
        "subset2_assign_if_else_rhs",
        "if_value_position_braces_at_indent",
        "if_call_argument_braces_at_indent",
        "if_else_anon_function_body_in_call",
        "if_else_block",
        "if_else_if_chain",
        "if_else_if_chain_long",
        "if_else_if_bare_flat",
        "if_else_if_block_propagation",
        "if_else_if_empty_branch",
        "if_nested_consequence",
        "if_nested_in_call_argument",
        "if_branch_trailing_comment_width",
        "if_statement_position_simple",
        "if_value_position_stays_flat",
        "if_value_position_nested_braces",
        "if_block_position_boundary",
        "if_else_interstitial_comment_block",
        "if_else_interstitial_comment_bare",
        "if_else_comment_forces_both_braces",
        "if_else_comment_wide_call_branches",
        "if_else_trailing_comment_after_block",
        "if_else_trailing_comment_preserves_order",
        "if_else_trailing_comment_after_bare",
        "if_condition_trailing_comment",
        "if_condition_comment_forms",
        "if_bare_then_leading_comment",
        "if_value_position_wide_bare_braces",
        "if_else_wide_bare_branches",
        "if_comment_wide_branch",
        "if_function_body_wide_if",
        "function_body_wide_if_condition",
        "function_body_wide_if_condition_nested",
        "inline_comment",
        "noop_assignment",
        "noop_if_else_block",
        "noop_comments",
        "noop_unary",
        "program",
        "quarto_code_annotations",
        "for_statement",
        "while_statement",
        "call_basic_and_holes",
        "expr_bracket_split_callee_paren",
        "expr_bracket_split_subscript",
        "expr_bracket_function_body_continuation",
        "call_dots_and_dotdoti",
        "call_user_line_breaks",
        "call_leading_holes",
        "call_comments_inside_holes",
        "call_comments_after_holes",
        "call_arg_comment_before_unary",
        "call_trailing_braced_expression",
        "call_trailing_inline_function",
        "call_comments_trailing_braced_expression",
        "call_named_args_without_rhs",
        "call_trailing_curly_curly",
        "call_empty_lines_between_args",
        "call_comments_basic",
        "call_hugging_basics",
        "call_single_arg_hug_overflow",
        "call_comments_sanity",
        "call_leading_holes_hugging",
        "call_subsetting_hugging",
        "tribble_table_basic",
        "tribble_table_numeric_alignment",
        "tribble_table_cells",
        "tribble_table_comments",
        "tribble_table_fallback",
        "function_definition_misc",
        "function_definition_autobracing",
        "function_definition_comments",
        "function_definition_user_requested_line_break",
        "function_definition_user_requested_line_break_followup",
        "function_bare_control_flow_body",
        "function_param_default_if_else",
        "function_param_comment_before_comma",
        "function_param_trailing_comment_space",
        "function_body_curly_curly",
        "subset_basic_and_holes",
        "subset_holes_trailing_function",
        "subset_dots_and_dotdoti",
        "subset_comments",
        "subset_user_requested_line_break",
        "subset_user_requested_line_break_leading_holes",
        "subset_comments_after_holes",
        "parenthesized_expression_basic",
        "paren_trailing_comment",
        "paren_own_line_comment",
        "braced_empty_and_basics",
        "braced_empty_function_definitions",
        "braced_empty_loops",
        "braced_empty_if_forms",
        "braced_curly_curly_basics",
        "braced_curly_curly_advanced",
        "braced_curly_curly_negative_non_symbol",
        "roxygen_loose_in_call",
        "roxygen_loose_in_subset",
        "roxygen_block_passthrough",
        "roxygen_section_block_flush",
        "roxygen_rd_block_macro_opener_flush",
        "roxygen_tag_folds_wrapped_inline_macro",
        "roxygen_section_folds_block_macro",
        "roxygen_rd_item_starts_own_line",
        "roxygen_rd_verbatim_not_hung",
        "roxygen_section_return_form1",
        "roxygen_section_return_form2",
        "roxygen_name_bearing_pulled_up",
        "roxygen_section_inline_if_fits",
        "roxygen_section_multiparagraph",
        "roxygen_section_null",
        "roxygen_token_list_join",
        "roxygen_atomic_value_overflow",
        "roxygen_indented_in_block",
        "roxygen_normalize_space",
        "roxygen_trailing_space",
        "roxygen_blank_trailing",
        "roxygen_tag_marker_space",
        "roxygen_multi_hash_kept",
        "roxygen_reflow_basic",
        "roxygen_md_inline_reflow",
        "roxygen_md_emphasis_multiline_reflow",
        "roxygen_reflow_join_short_lines",
        "roxygen_reflow_indented_in_function",
        "roxygen_reflow_multi_paragraph",
        "roxygen_reflow_blank_boundaries",
        "roxygen_reflow_atomic_inline_code",
        "roxygen_reflow_atomic_rd_macro",
        "roxygen_reflow_atomic_md_link",
        "roxygen_reflow_long_word",
        "roxygen_reflow_idempotent",
        "roxygen_bail_list",
        "roxygen_bail_code_fence",
        "roxygen_bail_examples_body",
        "roxygen_bail_rd_comment",
        "roxygen_tag_bail_rd_comment",
        "roxygen_rd_comment_md_reflows",
        "roxygen_bail_linkref_def",
        "roxygen_tag_bail_linkref_def",
        "roxygen_md_linkref_continuation_reflows",
        "roxygen_md_table",
        "roxygen_pipe_prose_reflow",
        "roxygen_md_heading",
        "roxygen_md_setext",
        "roxygen_md_setext_dash",
        "roxygen_md_blockquote",
        "roxygen_md_thematic_break",
        "roxygen_md_indented_code",
        "roxygen_md_html_verbatim",
        "roxygen_md_html_conditions",
        "roxygen_md_html_inline_forms",
        "roxygen_md_html_inline_multiline",
        "roxygen_md_code_multiline",
        "roxygen_md_html_cond7",
        "roxygen_md_html_block_value",
        "roxygen_md_block_value",
        "roxygen_md_list_lazy",
        "roxygen_md_list_loose",
        "roxygen_md_quote_break_value",
        "roxygen_tag_reflow_param",
        "roxygen_comment_gap",
        "roxygen_tag_reflow_return",
        "roxygen_tag_reflow_seealso",
        "roxygen_tag_reflow_absorb",
        "roxygen_tag_reflow_year_in_prose",
        "roxygen_reflow_year_not_list_item",
        "roxygen_tag_reflow_trailing_ordered_marker",
        "roxygen_bail_ordered_list",
        "roxygen_bail_folded_tag_list",
        "roxygen_bail_folded_section_list",
        "roxygen_tag_normalize_spacing",
        "roxygen_tag_separator_ws",
        "roxygen_tag_reflow_idempotent",
        "roxygen_tag_alone_passthrough",
        "roxygen_tag_examples_unchanged",
        "roxygen_examples_format",
        "roxygen_examples_multiline",
        "roxygen_examples_trailing_blank",
        "roxygen_examplesif_format",
        "roxygen_examples_dontrun_passthrough",
        "roxygen_examples_idempotent",
    ]
}

#[test]
fn formats_assignment_binary_and_paren_stably() {
    let input = "x<-(1+2)*3^4\n";
    let expected = "x <- (1 + 2) * 3^4\n";
    let formatted = format(input).expect("should format input");
    assert_eq!(formatted, expected);
    let reformatted = format(&formatted).expect("should remain formatable");
    assert_eq!(reformatted, expected);
}

#[test]
fn formats_if_else_blocks_with_comments_and_strings() {
    let input = "if(x){# keep\nmsg<-'a+b'\n}else{y<-1+2}\n";
    let expected = "if (x) {\n  # keep\n  msg <- 'a+b'\n} else {\n  y <- 1 + 2\n}\n";
    let formatted = format(input).expect("should format if/else blocks");
    assert_eq!(formatted, expected);
}

#[test]
fn preserves_comment_only_lines() {
    let input = "x<-1\n# untouched\n";
    let expected = "x <- 1\n# untouched\n";
    let formatted = format(input).expect("should format and preserve comments");
    assert_eq!(formatted, expected);
}

#[test]
fn formats_at_slot_extraction_like_dollar() {
    let input = "x @ y\n";
    let expected = "x@y\n";
    let formatted = format(input).expect("@ slot extraction should format");
    assert_eq!(formatted, expected);
}

#[test]
fn explicit_default_style_matches_default_format() {
    let input = "if(x){y<-1+2}else{z<-3}\n";
    let implicit = format(input).expect("default format should succeed");
    let explicit = format_with_style(input, FormatStyle::default())
        .expect("format_with_style default should succeed");
    assert_eq!(implicit, explicit);
}

#[test]
fn wraps_binary_expression_when_width_is_exceeded() {
    let input = "alpha <- beta + gamma_delta\n";
    let style = FormatStyle {
        line_width: 17,
        indent_width: 2,
        ..FormatStyle::default()
    };
    let expected = "alpha <- beta +\n  gamma_delta\n";
    let formatted = format_with_style(input, style).expect("format should succeed");
    assert_eq!(formatted, expected);

    let reformatted = format_with_style(&formatted, style).expect("reformat should succeed");
    assert_eq!(reformatted, expected);
}

#[test]
fn formats_magrittr_pipe_like_native_pipe() {
    let input = "df %>% foo(a = 1) %>% bar(b = 2, c = 3)\n";
    let expected = "df %>%\n  foo(a = 1) %>%\n  bar(b = 2, c = 3)\n";
    let formatted = format(input).expect("format should succeed");
    assert_eq!(formatted, expected);

    let reformatted = format(&formatted).expect("reformat should succeed");
    assert_eq!(reformatted, expected);
}

#[test]
fn wraps_call_arguments_when_width_is_exceeded() {
    let input = "call(first_arg, second_argument, third)\n";
    let style = FormatStyle {
        line_width: 22,
        indent_width: 2,
        ..FormatStyle::default()
    };
    let expected = "call(\n  first_arg,\n  second_argument,\n  third\n)\n";
    let formatted = format_with_style(input, style).expect("format should succeed");
    assert_eq!(formatted, expected);

    let reformatted = format_with_style(&formatted, style).expect("reformat should succeed");
    assert_eq!(reformatted, expected);
}

#[test]
fn preserves_trailing_comments_when_wrapping_calls() {
    let input = "fn_name(argument, second) # keep\n";
    let style = FormatStyle {
        line_width: 18,
        indent_width: 2,
        ..FormatStyle::default()
    };
    let expected = "fn_name(\n  argument,\n  second\n) # keep\n";
    let formatted = format_with_style(input, style).expect("format should succeed");
    assert_eq!(formatted, expected);

    let reformatted = format_with_style(&formatted, style).expect("reformat should succeed");
    assert_eq!(reformatted, expected);
}

#[test]
fn block_contents_are_width_aware() {
    let input = "if (x) { total <- alpha + gamma_delta }\n";
    let style = FormatStyle {
        line_width: 20,
        indent_width: 2,
        ..FormatStyle::default()
    };
    let expected = "if (x) {\n  total <- alpha +\n    gamma_delta\n}\n";
    let formatted = format_with_style(input, style).expect("format should succeed");
    assert_eq!(formatted, expected);

    let reformatted = format_with_style(&formatted, style).expect("reformat should succeed");
    assert_eq!(reformatted, expected);
}

#[test]
fn formats_user_operator_binary_expr() {
    let input = "1:3 %in% 1:5\n";
    let expected = "1:3 %in% 1:5\n";
    let formatted =
        format(input).expect("user operators should be formatted as binary expressions");
    assert_eq!(formatted, expected);
}

#[test]
fn formatter_fixtures_match_expected_and_snapshots() {
    for name in fixture_names() {
        let input = fixture_text(name, "input.R");
        let expected = fixture_text(name, "expected.R");
        let formatted = format_verified(&input).unwrap_or_else(|err| {
            panic!("failed to format and verify fixture {name}: {err}");
        });

        assert_eq!(formatted, expected, "formatted output mismatch for {name}");
        assert_snapshot!(format!("{name}_formatted"), formatted);
    }
}

#[test]
fn parse_format_fixtures_are_stable_and_parseable() {
    for name in fixture_names() {
        let input = fixture_text(name, "input.R");
        let expected = fixture_text(name, "expected.R");

        let parsed_input = parse(&input);
        assert!(
            parsed_input.diagnostics.is_empty(),
            "fixture {name} input should be parseable, got diagnostics: {:#?}",
            parsed_input.diagnostics
        );

        let formatted = format(&input).unwrap_or_else(|err| {
            panic!("failed to format fixture {name}: {err}");
        });
        assert_eq!(
            formatted, expected,
            "formatted output mismatch for integration fixture {name}"
        );

        let reparsed = parse(&formatted);
        assert!(
            reparsed.diagnostics.is_empty(),
            "fixture {name} formatted output should be parseable, got diagnostics: {:#?}",
            reparsed.diagnostics
        );
        assert_eq!(
            reconstruct(&formatted),
            formatted,
            "fixture {name} formatted output should round-trip losslessly"
        );

        let reformatted = format(&formatted).unwrap_or_else(|err| {
            panic!("failed to reformat fixture {name}: {err}");
        });
        assert_eq!(
            reformatted, formatted,
            "fixture {name} formatting should be idempotent"
        );
    }
}
