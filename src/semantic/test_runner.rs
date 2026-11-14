//! Bindings supplied by conventional test runners, independently of package
//! attachment. Keep these separate from exports: runners supply only a subset
//! of their package's exports and can add names that the package never exports.

use std::path::Path;
use std::sync::LazyLock;

use smol_str::SmolStr;

/// Tinytest 1.4.3's `add_locally_masked_functions` and `run_test_file` supply
/// these bindings without attaching tinytest. Pinned against the actual test
/// evaluation environment; see <https://github.com/cran/tinytest/blob/1.4.3/R/tinytest.R>.
///
/// The `check*` aliases assume the runner's default `tt.RUnitStyle = TRUE`.
/// External R options are not available to static analysis. Extensions loaded
/// through `using()` are not guessed. The runner also masks `options` and
/// `Sys.setenv`, which already resolve through base R.
static TINYTEST_BINDINGS: LazyLock<Vec<SmolStr>> = LazyLock::new(|| {
    [
        "expect_equal",
        "expect_equivalent",
        "expect_length",
        "expect_true",
        "expect_false",
        "expect_inherits",
        "expect_null",
        "expect_message",
        "expect_warning",
        "expect_error",
        "expect_stdout",
        "expect_identical",
        "expect_silent",
        "expect_equal_to_reference",
        "expect_equivalent_to_reference",
        "expect_match",
        "exit_file",
        "exit_if_not",
        "ignore",
        "at_home",
        "using",
        "report_side_effects",
        "checkEqual",
        "checkEquivalent",
        "checkLength",
        "checkTrue",
        "checkFalse",
        "checkInherits",
        "checkNull",
        "checkMessage",
        "checkWarning",
        "checkError",
        "checkStdout",
        "checkIdentical",
        "checkSilent",
        // Tinytest capitalizes only the first word after `expect_`.
        "checkEqual_to_reference",
        "checkEquivalent_to_reference",
        "checkMatch",
    ]
    .into_iter()
    .map(SmolStr::new)
    .collect()
});

/// Bindings implied by a file's conventional runner location. Like testthat's
/// implicit attachment, this matches path structure without filesystem reads,
/// so standalone buffers and salsa snapshots use exactly the same convention.
/// Only direct `inst/tinytest/` members are test files; nested fixture sources
/// do not inherit the runner environment just by living below that directory.
pub(crate) fn implicit_bindings(path: &Path) -> &'static [SmolStr] {
    let parent = path.parent();
    if parent
        .and_then(Path::file_name)
        .is_some_and(|name| name == "tinytest")
        && parent
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .is_some_and(|name| name == "inst")
    {
        &TINYTEST_BINDINGS
    } else {
        &[]
    }
}
