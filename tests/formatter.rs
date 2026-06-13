use arity::formatter::{FormatStyle, format, format_with_style};
use arity::parser::{parse, reconstruct};
use insta::assert_snapshot;
use std::io::Write;
use std::process::{Command, Stdio};
use std::{fs, path::Path};
use tempfile::tempdir;

fn run_cli_no_stdin<const N: usize>(args: [&str; N]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_arity"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to run cli")
}

fn run_cli<const N: usize>(args: [&str; N], stdin_input: &str) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_arity"));
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn arity cli");
    let mut stdin = child.stdin.take().expect("failed to open stdin");
    stdin
        .write_all(stdin_input.as_bytes())
        .expect("failed to write stdin");
    drop(stdin);

    child.wait_with_output().expect("failed to wait for cli")
}
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
        "if_else_block",
        "if_else_if_chain",
        "if_else_if_chain_long",
        "if_else_if_bare_flat",
        "if_else_if_block_propagation",
        "if_else_if_empty_branch",
        "if_nested_consequence",
        "if_nested_in_call_argument",
        "if_statement_position_simple",
        "if_value_position_stays_flat",
        "if_value_position_nested_braces",
        "if_block_position_boundary",
        "if_else_interstitial_comment_block",
        "if_else_interstitial_comment_bare",
        "if_else_trailing_comment_after_block",
        "if_else_trailing_comment_after_bare",
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
        "for_statement",
        "while_statement",
        "call_basic_and_holes",
        "call_dots_and_dotdoti",
        "call_user_line_breaks",
        "call_leading_holes",
        "call_comments_inside_holes",
        "call_comments_after_holes",
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
        "function_definition_misc",
        "function_definition_autobracing",
        "function_definition_comments",
        "function_definition_user_requested_line_break",
        "function_definition_user_requested_line_break_followup",
        "function_bare_control_flow_body",
        "function_param_default_if_else",
        "function_body_curly_curly",
        "subset_basic_and_holes",
        "subset_holes_trailing_function",
        "subset_dots_and_dotdoti",
        "subset_comments",
        "subset_user_requested_line_break",
        "subset_user_requested_line_break_leading_holes",
        "subset_comments_after_holes",
        "parenthesized_expression_basic",
        "braced_empty_and_basics",
        "braced_empty_function_definitions",
        "braced_empty_loops",
        "braced_empty_if_forms",
        "braced_curly_curly_basics",
        "braced_curly_curly_advanced",
        "braced_curly_curly_negative_non_symbol",
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
    };
    let expected = "if (x) {\n  total <- alpha +\n    gamma_delta\n}\n";
    let formatted = format_with_style(input, style).expect("format should succeed");
    assert_eq!(formatted, expected);

    let reformatted = format_with_style(&formatted, style).expect("reformat should succeed");
    assert_eq!(reformatted, expected);
}

#[test]
fn cli_format_verify_formats_stdin() {
    let output = run_cli(["format", "--verify"], "if(x){y<-1+2}else{z<-3}\n");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "if (x) {\n  y <- 1 + 2\n} else {\n  z <- 3\n}\n"
    );
}

#[test]
fn cli_format_writes_single_file_in_place() {
    let dir = tempdir().expect("failed to create temp dir");
    let file = dir.path().join("in_place.R");
    std::fs::write(&file, "x<-1+2\n").expect("failed to write input file");

    let output = run_cli_no_stdin([
        "format",
        file.to_str().expect("temp file path should be utf-8"),
    ]);
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let after = std::fs::read_to_string(&file).expect("failed to read formatted file");
    assert_eq!(after, "x <- 1 + 2\n");
}

#[test]
fn cli_format_writes_directory_files_in_place() {
    let dir = tempdir().expect("failed to create temp dir");
    let a = dir.path().join("a.R");
    let b = dir.path().join("sub").join("b.R");
    std::fs::create_dir_all(b.parent().expect("subdir parent should exist"))
        .expect("failed to create nested dir");
    std::fs::write(&a, "x<-1+2\n").expect("failed to write a.R");
    std::fs::write(&b, "y<-3+4\n").expect("failed to write b.R");

    let output = run_cli_no_stdin([
        "format",
        dir.path().to_str().expect("temp dir path should be utf-8"),
    ]);
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        std::fs::read_to_string(&a).expect("failed to read a.R"),
        "x <- 1 + 2\n"
    );
    assert_eq!(
        std::fs::read_to_string(&b).expect("failed to read b.R"),
        "y <- 3 + 4\n"
    );
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
fn cli_format_check_reports_changed_files() {
    let dir = tempdir().expect("failed to create temp dir");
    let changed = dir.path().join("changed.R");
    let unchanged = dir.path().join("unchanged.R");

    std::fs::write(&changed, "x<-1+2\n").expect("failed to write changed file");
    std::fs::write(&unchanged, "x <- 1 + 2\n").expect("failed to write unchanged file");

    let output = run_cli_no_stdin([
        "format",
        "--check",
        dir.path().to_str().expect("temp dir path should be utf-8"),
    ]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("would reformat:"));
    assert!(stderr.contains("changed.R"));
    assert!(!stderr.contains("unchanged.R"));
}

#[test]
fn cli_format_check_succeeds_for_unchanged_files() {
    let dir = tempdir().expect("failed to create temp dir");
    let unchanged = dir.path().join("unchanged.R");
    std::fs::write(&unchanged, "x <- 1 + 2\n").expect("failed to write unchanged file");

    let output = run_cli_no_stdin([
        "format",
        "--check",
        dir.path().to_str().expect("temp dir path should be utf-8"),
    ]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
}

#[test]
fn cli_format_check_requires_paths() {
    let output = run_cli_no_stdin(["format", "--check"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--check requires at least one input path"));
}

#[test]
fn cli_format_check_disallows_verify() {
    let output = run_cli_no_stdin(["format", "--check", "--verify", "."]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--verify cannot be combined with --check"));
}

#[test]
fn formatter_fixtures_match_expected_and_snapshots() {
    for name in fixture_names() {
        let input = fixture_text(name, "input.R");
        let expected = fixture_text(name, "expected.R");
        let formatted = format(&input).unwrap_or_else(|err| {
            panic!("failed to format fixture {name}: {err}");
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
