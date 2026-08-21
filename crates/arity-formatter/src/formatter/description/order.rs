//! Canonical field order.
//!
//! Transcribed from `desc:::field_order`: a fixed head, then everything else
//! sorted, then the `Collate` family last. Matching `desc` is the point — it is
//! what `usethis` and `devtools` users already see — but `desc` sorts with R's
//! locale-dependent `sort()`, which is why the collation below is spelled out
//! here rather than inherited. A formatter whose output depends on `LC_COLLATE`
//! is not deterministic.

use std::cmp::Ordering;

/// Fields with a fixed position, in that position's order.
const FIRST: [&str; 20] = [
    "Type",
    "Package",
    "Title",
    "Version",
    "Date",
    "Authors@R",
    "Author",
    "Maintainer",
    "Description",
    "License",
    "URL",
    "BugReports",
    "Depends",
    "Imports",
    "Suggests",
    "Enhances",
    "LinkingTo",
    "VignetteBuilder",
    "RdMacros",
    "Remotes",
];

/// Standard fields without a fixed position, in deterministic collation order.
///
/// The set comes from R's `tools:::.get_standard_DESCRIPTION_fields()` (R
/// 4.6.1), plus `RoxygenNote`, which is ubiquitous package metadata. `Remotes`
/// is likewise included in [`FIRST`]. This is also the candidate set for
/// consumers that need to recognize a likely misspelling. `Config/*` and other
/// extension fields remain legal; the list says what is standard, not what is
/// allowed.
const MIDDLE: [&str; 49] = [
    "Acknowledgements",
    "Acknowledgments",
    "Additional_repositories",
    "Archs",
    "Biarch",
    "biocViews",
    "BuildKeepEmpty",
    "BuildManual",
    "BuildResaveData",
    "BuildVignettes",
    "Built",
    "ByteCompile",
    "Classification/ACM",
    "Classification/ACM-2012",
    "Classification/JEL",
    "Classification/MSC",
    "Classification/MSC-2010",
    "Contact",
    "Copyright",
    "Date/Publication",
    "Encoding",
    "KeepSource",
    "Language",
    "LastChangedDate",
    "LastChangedRevision",
    "LazyData",
    "LazyDataCompression",
    "LazyLoad",
    "License_is_FOSS",
    "License_restricts_use",
    "MailingList",
    "MD5sum",
    "NeedsCompilation",
    "Note",
    "OS_type",
    "Packaged",
    "Path",
    "Priority",
    "RcmdrModels",
    "RcppModules",
    "Repository",
    "Revision",
    "Roxygen",
    "RoxygenNote",
    "StagedInstall",
    "SysDataCompression",
    "SystemRequirements",
    "UseLTO",
    "ZipData",
];

/// Fields pinned to the end, in that order.
const LAST: [&str; 3] = ["Collate", "Collate.windows", "Collate.unix"];

/// Standard `DESCRIPTION` field names in canonical formatter order.
pub fn field_names() -> impl Iterator<Item = &'static str> + Clone {
    FIRST.into_iter().chain(MIDDLE).chain(LAST)
}

/// The bucket a field sorts into, and its index within a positional bucket.
/// Bucket 1 is "everything else", which sorts by [`collate`] instead.
fn rank(name: &str) -> (u8, usize) {
    if let Some(index) = FIRST.iter().position(|candidate| *candidate == name) {
        return (0, index);
    }
    if let Some(index) = LAST.iter().position(|candidate| *candidate == name) {
        return (2, index);
    }
    (1, 0)
}

/// Compare two field names for canonical order.
pub(super) fn compare_fields(left: &str, right: &str) -> Ordering {
    let (left_bucket, left_index) = rank(left);
    let (right_bucket, right_index) = rank(right);
    left_bucket
        .cmp(&right_bucket)
        .then_with(|| match left_bucket {
            1 => collate(left, right),
            _ => left_index.cmp(&right_index),
        })
}

/// The one collation: ASCII case-insensitive, ties broken by byte order.
///
/// Case-insensitive because that is what R's `en_US` collation does in practice,
/// and what the CRAN corpus shows: roxygen2's `Imports` reads
/// `pkgload, R6, rlang`, with `R6` between two lowercase names. Byte order alone
/// would hoist every capitalized package to the front.
pub(super) fn collate(left: &str, right: &str) -> Ordering {
    let folded = left
        .bytes()
        .map(|byte| byte.to_ascii_lowercase())
        .cmp(right.bytes().map(|byte| byte.to_ascii_lowercase()));
    folded.then_with(|| left.cmp(right))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut names: Vec<&str>) -> Vec<&str> {
        names.sort_by(|left, right| compare_fields(left, right));
        names
    }

    #[test]
    fn the_fixed_head_keeps_its_written_order() {
        assert_eq!(
            sorted(vec!["Version", "Package", "Title", "Type"]),
            vec!["Type", "Package", "Title", "Version"]
        );
    }

    #[test]
    fn unknown_fields_sort_between_the_head_and_collate() {
        assert_eq!(
            sorted(vec![
                "Collate",
                "Encoding",
                "Package",
                "Config/Needs/website"
            ]),
            vec!["Package", "Config/Needs/website", "Encoding", "Collate"]
        );
    }

    #[test]
    fn the_collate_family_comes_last_in_its_own_order() {
        assert_eq!(
            sorted(vec!["Collate.unix", "Collate", "Collate.windows"]),
            vec!["Collate", "Collate.windows", "Collate.unix"]
        );
    }

    #[test]
    fn collation_is_case_insensitive_and_locale_free() {
        assert_eq!(collate("R6", "rlang"), Ordering::Less);
        assert_eq!(collate("pkgload", "R6"), Ordering::Less);
        assert_eq!(collate("MASS", "zoo"), Ordering::Less);
        // A pure byte comparison would put every capitalized name first.
        assert_eq!(collate("Rcpp", "stats"), Ordering::Less);
        // Ties fall back to byte order so the sort is total.
        assert_eq!(collate("abc", "ABC"), Ordering::Greater);
    }

    #[test]
    fn exported_field_names_are_in_canonical_order() {
        let names: Vec<_> = field_names().collect();
        assert!(
            names
                .windows(2)
                .all(|pair| { compare_fields(pair[0], pair[1]) == Ordering::Less })
        );
    }
}
