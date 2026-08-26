//! Integration coverage for the persistent `format --check` "already-formatted"
//! cache: an already-formatted file is recorded as a fixed point (and persists
//! across a fresh load), while a file that needs reformatting is never recorded.

use std::fs;
use std::path::PathBuf;

use arity::file_discovery::ExcludeFilter;
use arity::formatter::{CacheKey, FormatCache, FormatStyle, check_paths_with_style_cached};

fn write_file(dir: &std::path::Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).unwrap();
    path
}

#[test]
fn already_formatted_file_is_recorded_and_persists() {
    let src = tempfile::tempdir().unwrap();
    let cache_root = tempfile::tempdir().unwrap();
    let style = FormatStyle::default();
    let clean = "x <- 1\n";
    let file = write_file(src.path(), "clean.R", clean);

    let mut cache = FormatCache::load(cache_root.path(), &style);
    let result = check_paths_with_style_cached(
        std::slice::from_ref(&file),
        style,
        &ExcludeFilter::none(),
        true,
        Some(&mut cache),
    )
    .unwrap();

    // A clean file: no diff, and recorded as a fixed point.
    assert!(result.changed_files.is_empty());
    assert_eq!(result.checked_files, 1);
    assert!(cache.is_fixed_point(CacheKey::r(clean, false)));

    // The record was persisted: a fresh load off disk still sees it, and a
    // second run serves it from cache (still clean).
    let mut reloaded = FormatCache::load(cache_root.path(), &style);
    assert!(reloaded.is_fixed_point(CacheKey::r(clean, false)));
    let second = check_paths_with_style_cached(
        std::slice::from_ref(&file),
        style,
        &ExcludeFilter::none(),
        true,
        Some(&mut reloaded),
    )
    .unwrap();
    assert!(second.changed_files.is_empty());
}

#[test]
fn unformatted_file_is_never_recorded() {
    let src = tempfile::tempdir().unwrap();
    let cache_root = tempfile::tempdir().unwrap();
    let style = FormatStyle::default();
    // Needs reformatting (`x<-1` -> `x <- 1`).
    let dirty = "x<-1\n";
    let file = write_file(src.path(), "dirty.R", dirty);

    let mut cache = FormatCache::load(cache_root.path(), &style);
    let result = check_paths_with_style_cached(
        std::slice::from_ref(&file),
        style,
        &ExcludeFilter::none(),
        true,
        Some(&mut cache),
    )
    .unwrap();

    assert_eq!(result.changed_files.len(), 1);
    // The unformatted content must not be cached as a fixed point.
    assert!(!cache.is_fixed_point(CacheKey::r(dirty, false)));
    // A fresh load confirms nothing about it was persisted.
    let reloaded = FormatCache::load(cache_root.path(), &style);
    assert!(!reloaded.is_fixed_point(CacheKey::r(dirty, false)));
}

#[test]
fn file_with_outdated_directive_is_never_recorded() {
    let src = tempfile::tempdir().unwrap();
    let cache_root = tempfile::tempdir().unwrap();
    let style = FormatStyle::default();
    let content = "# arity-format skip: legacy workaround\nx <- 1\n";
    let file = write_file(src.path(), "outdated.R", content);

    let mut cache = FormatCache::load(cache_root.path(), &style);
    let result = check_paths_with_style_cached(
        std::slice::from_ref(&file),
        style,
        &ExcludeFilter::none(),
        true,
        Some(&mut cache),
    )
    .unwrap();

    assert!(result.changed_files.is_empty());
    assert_eq!(result.outdated_directives.len(), 1);
    assert!(!cache.is_fixed_point(CacheKey::r(content, false)));
}

#[test]
fn no_cache_run_writes_nothing() {
    let src = tempfile::tempdir().unwrap();
    let cache_root = tempfile::tempdir().unwrap();
    let style = FormatStyle::default();
    let file = write_file(src.path(), "clean.R", "x <- 1\n");

    // `None` cache mirrors `--no-cache`: the run works and touches no cache dir.
    let result = check_paths_with_style_cached(
        std::slice::from_ref(&file),
        style,
        &ExcludeFilter::none(),
        true,
        None,
    )
    .unwrap();
    assert!(result.changed_files.is_empty());

    // Nothing was written under the cache root.
    let format_dir = cache_root.path().join("format");
    assert!(!format_dir.exists());
}

#[test]
fn the_cache_never_crosses_grammars() {
    // A lone comment line is a fixed point of both grammars. Without the
    // grammar in the key, formatting one would report the other clean.
    let src = tempfile::tempdir().unwrap();
    let cache_root = tempfile::tempdir().unwrap();
    let style = FormatStyle::default();

    // Byte-identical content, so only the grammar distinguishes them.
    const SHARED: &str = "# a comment\n";
    std::fs::create_dir(src.path().join("R")).unwrap();
    let r_file = write_file(src.path(), "R/note.R", SHARED);
    let description = write_file(src.path(), "DESCRIPTION", SHARED);

    // Prime the cache with the `.R` file only.
    let mut cache = FormatCache::load(cache_root.path(), &style);
    let result = check_paths_with_style_cached(
        std::slice::from_ref(&r_file),
        style,
        &ExcludeFilter::none(),
        true,
        Some(&mut cache),
    )
    .unwrap();
    assert!(result.changed_files.is_empty());
    assert!(cache.is_fixed_point(CacheKey::r(SHARED, false)));
    assert!(
        !cache.is_fixed_point(CacheKey::dcf(SHARED)),
        "an .R fixed point must not answer for a DESCRIPTION"
    );

    // So the DESCRIPTION is genuinely formatted rather than skipped on the
    // `.R` file's entry, and records its own.
    check_paths_with_style_cached(
        std::slice::from_ref(&description),
        style,
        &ExcludeFilter::none(),
        true,
        Some(&mut cache),
    )
    .unwrap();
    assert!(cache.is_fixed_point(CacheKey::dcf(SHARED)));
}
