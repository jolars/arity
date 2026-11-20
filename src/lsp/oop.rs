//! Editor consumers share the project class index, including live-buffer overlays.

use super::*;
use crate::config::CompatVersion;
use crate::project::classes::{ClassIndex, file_class_defs};

mod navigation;
pub(super) mod presentation;
pub(super) use navigation::{
    NameNavigation, class_candidate, implementation_locations, name_locations,
    namespace_callable_references,
};

#[derive(Clone, Copy)]
pub(super) struct ReadContext<'a> {
    pub snapshot: &'a Analysis,
    pub path: &'a Path,
    pub s7_floor: Option<&'a CompatVersion>,
}

pub(super) fn with_live_model<T>(
    root: &SyntaxNode,
    context: Option<ReadContext<'_>>,
    compute: impl Fn(&Analysis, &SemanticModel, &Path) -> T,
) -> T {
    let path = context.map_or(Path::new("buffer.R"), |context| context.path);
    let floor = context.and_then(|context| context.s7_floor);
    if let Some(ReadContext { snapshot, .. }) = context {
        let result = salsa::Cancelled::catch(AssertUnwindSafe(|| {
            let model = snapshot.semantic_model_in_with_floor(path, root, floor);
            compute(snapshot, &model, path)
        }));
        if let Ok(result) = result {
            return result;
        }
    }
    let snapshot = IncrementalDatabase::default().snapshot();
    let model = snapshot.semantic_model_in_with_floor(path, root, floor);
    compute(&snapshot, &model, path)
}

pub(super) fn with_class_index<T>(
    root: &SyntaxNode,
    context: Option<ReadContext<'_>>,
    compute: impl Fn(&ClassIndex, &SemanticModel, &Path) -> T,
) -> T {
    with_live_model(root, context, |snapshot, model, path| {
        let facts = file_class_defs(model, root);
        let index = snapshot.class_index_in(path, &facts, root, model);
        compute(&index, model, path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::PackageInfo;
    use crate::text::TextBuffer;
    use std::collections::BTreeSet;

    fn implementations_at(
        snapshot: &Analysis,
        path: &Path,
        source: &str,
        offset: usize,
    ) -> Vec<Location> {
        let uri = uri::from_path(path).unwrap();
        let buffer = TextBuffer::from(source);
        let encoding = PositionEncoding::Utf16;
        let position = buffer.line_index().byte_to_position(offset, encoding);
        match implementation_via_db(snapshot, path, &uri, &buffer, position, encoding, None) {
            Some(GotoDefinitionResponse::Scalar(location)) => vec![location],
            Some(GotoDefinitionResponse::Array(locations)) => locations,
            None => Vec::new(),
            other => panic!("expected locations, got {other:?}"),
        }
    }

    fn implementation_lines(locations: Vec<Location>) -> Vec<u32> {
        let mut lines: Vec<_> = locations
            .into_iter()
            .map(|loc| loc.range.start.line)
            .collect();
        lines.sort_unstable();
        lines
    }

    fn lazy_index(
        package: &crate::rindex::schema::PackageIndex,
        directory: &Path,
    ) -> IndexedProvider {
        let cache = crate::rindex::cache::Cache::new(directory.to_path_buf());
        cache.write_package(package).unwrap();
        IndexedProvider::from_cache_lazy(&cache)
    }

    #[test]
    fn s3_reassignments_preserve_lexical_references() {
        let source = "describe <- function(x) UseMethod('describe')
describe.example <- function(x) x
describe.example <- 2
describe.example(1)
describe(1)
";
        let path = ws_path("a.R");
        let snapshot = rename_workspace(source, "");
        let uri = uri::from_path(&path).unwrap();
        let buffer = TextBuffer::from(source);
        let references = |line, include_declaration| {
            implementation_lines(
                references_via_db(
                    &snapshot,
                    &path,
                    &uri,
                    &buffer,
                    Position::new(line, 1),
                    include_declaration,
                    PositionEncoding::Utf16,
                    &[],
                )
                .unwrap(),
            )
        };
        assert_eq!(references(3, true), [1, 2, 3]);
        assert_eq!(references(4, false), [4]);
        let offset = source.find("describe.example(1)").unwrap() + 1;
        assert!(implementations_at(&snapshot, &path, source, offset).is_empty());
    }

    #[test]
    fn s7_generic_declaration_is_excluded_from_use_only_references() {
        let source = r#"describe <- S7::new_generic("describe", "x")
describe(1)
"#;
        let path = ws_path("a.R");
        let snapshot = rename_workspace(source, "");
        let references = references_via_db(
            &snapshot,
            &path,
            &uri::from_path(&path).unwrap(),
            &TextBuffer::from(source),
            Position::new(1, 1),
            false,
            PositionEncoding::Utf16,
            &[],
        )
        .unwrap();
        assert_eq!(implementation_lines(references), [1]);
    }

    #[test]
    fn is_class_navigation_accepts_formal() {
        let source = "methods::setClass('Example')
methods::isClass('Example', formal = TRUE)
methods::isClass('Example', TRUE)
";
        let path = ws_path("a.R");
        let snapshot = rename_workspace(source, "");
        let result = references_via_db(
            &snapshot,
            &path,
            &uri::from_path(&path).unwrap(),
            &TextBuffer::from(source),
            Position::new(0, 20),
            false,
            PositionEncoding::Utf16,
            &[],
        )
        .unwrap();
        assert_eq!(implementation_lines(result), [1, 2]);
    }

    #[test]
    fn external_implementations_follow_registration_order() {
        let source = r#"methods::setClass("Example")
methods::setMethod("show", "Example", function(object) 1)
methods::setMethod("show", "Example", function(object) 2)
methods::show(x)
methods::removeMethod("show", "Example")
methods::show(x)
base::registerS3method("print", "example", function(x, ...) 1)
base::registerS3method("print", "example", function(x, ...) 2)
base::print(x)
"#;
        let path = ws_path("a.R");
        let snapshot = rename_workspace(source, "");
        let show = |offset| implementations_at(&snapshot, &path, source, offset);
        assert_eq!(
            implementation_lines(show(source.find("show(x)").unwrap() + 1)),
            [2]
        );
        assert!(show(source.rfind("show(x)").unwrap() + 1).is_empty());
        assert_eq!(
            implementation_lines(show(source.find("print(x)").unwrap() + 1)),
            [7]
        );
    }

    #[test]
    fn imported_non_r6_factory_does_not_define_r6_members() {
        let consumer = ws_path("consumer");
        let provider = ws_path("provider");
        let path = consumer.join("R/source.R");
        let source = r#"A <- R6Class("A", public = list(read = function() 1))
"#;
        let mut db = IncrementalDatabase::default();
        let file = db.upsert_file(&path, source);
        let factory = db.upsert_file(
            &provider.join("R/factory.R"),
            "R6Class <- function(...) NULL",
        );
        db.set_workspace_members(vec![file, factory], vec![ws_root()]);
        db.upsert_description(&consumer, "Package: consumer\nImports: provider\n");
        db.upsert_description(&provider, "Package: provider\n");
        db.set_package_metadata(vec![
            PackageInfo {
                root: consumer,
                namespace: Some("importFrom(provider, R6Class)\n".into()),
                dir_sources: BTreeSet::new(),
                native_routines: BTreeSet::new(),
            },
            PackageInfo {
                root: provider,
                namespace: Some("export(R6Class)\n".into()),
                dir_sources: BTreeSet::new(),
                native_routines: BTreeSet::new(),
            },
        ]);
        let buffer = TextBuffer::from(source);
        let encoding = PositionEncoding::Utf16;
        let position = buffer
            .line_index()
            .byte_to_position(source.find("read").unwrap() + 1, encoding);
        assert!(
            definition_via_db(
                &db.snapshot(),
                &path,
                &uri::from_path(&path).unwrap(),
                &buffer,
                position,
                encoding,
                None,
            )
            .is_none()
        );
    }

    #[test]
    fn implementation_follows_private_r6_overrides() {
        let source = r#"Parent <- R6::R6Class("Parent", private = list(read = function() 1))
Child <- R6::R6Class("Child", inherit = Parent, private = list(read = function() 2))
"#;
        let path = ws_path("a.R");
        let snapshot = rename_workspace(source, "");
        let offset = source.find("read =").unwrap() + 1;
        assert_eq!(
            implementation_lines(implementations_at(&snapshot, &path, source, offset)),
            [1]
        );
    }

    #[test]
    fn implementation_finds_member_overrides_and_visible_external_methods() {
        let source = r#"Parent <- R6::R6Class("Parent", public = list(read = function() 1))
Child <- R6::R6Class("Child", inherit = Parent, public = list(read = function() 2))
Other <- R6::R6Class("Other", public = list(read = function() 3))
print.example <- function(x, ...) x
base::print(x)
"#;
        let path = ws_path("a.R");
        let snapshot = rename_workspace(source, "print.unrelated <- function(x, ...) x");
        for (name, line) in [("read = ", 1), ("print(x)", 3)] {
            let offset = source.find(name).unwrap() + 1;
            assert_eq!(
                implementation_lines(implementations_at(&snapshot, &path, source, offset)),
                [line],
                "{name}"
            );
        }
    }

    #[test]
    fn implementation_uses_known_generics_from_installed_metadata() {
        use crate::rindex::harvest::{HarvestOptions, harvest_package};
        use crate::rindex::schema::ObjectSystem;
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rindex/oopfixture");
        let mut package = harvest_package(&fixture, HarvestOptions::default(), 0).unwrap();
        let generic = package
            .objects
            .generics
            .iter_mut()
            .find(|generic| generic.origin.binding == "area")
            .unwrap();
        generic.system = ObjectSystem::S3;
        let source = "library(oopfixture)\nbase::.S3method('area', 'example', function(x) x)\noopfixture::area(x)\n";
        let path = ws_path("a.R");
        let mut db = IncrementalDatabase::default();
        let file = db.upsert_file(&path, source);
        db.set_workspace_members(vec![file], vec![ws_root()]);
        let offset = source.find("area(x)").unwrap() + 1;
        assert!(implementations_at(&db.snapshot(), &path, source, offset).is_empty());
        let directory = tempfile::tempdir().unwrap();
        db.set_library_index(lazy_index(&package, directory.path()));
        assert_eq!(
            implementation_lines(implementations_at(&db.snapshot(), &path, source, offset)),
            [1]
        );
    }

    #[test]
    fn implementation_finds_visible_methods_of_installed_s7_generics() {
        use crate::rindex::harvest::{HarvestOptions, harvest_package};
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rindex/oopfixture");
        let package = harvest_package(&fixture, HarvestOptions::default(), 0).unwrap();
        let source = "oopfixture::measure(x)\nS7::method(oopfixture::measure, S7::class_integer) <- function(x, scale = 1) x\noopfixture::measure(x)\n";
        let path = ws_path("a.R");
        let mut db = IncrementalDatabase::default();
        db.set_library_index(IndexedProvider::from_indices([package]));
        let file = db.upsert_file(&path, source);
        db.set_workspace_members(vec![file], vec![ws_root()]);
        let before = source.find("measure(x)").unwrap() + 1;
        let after = source.rfind("measure(x)").unwrap() + 1;
        let check = |db: &IncrementalDatabase, path| {
            let snapshot = db.snapshot();
            assert!(implementations_at(&snapshot, path, source, before).is_empty());
            assert_eq!(
                implementation_lines(implementations_at(&snapshot, path, source, after)),
                [1]
            );
        };
        check(&db, &path);

        let package = ws_path("consumer");
        let path = package.join("R/source.R");
        let file = db.upsert_file(&path, source);
        db.set_workspace_members(vec![file], vec![ws_root()]);
        let set_package = |db: &mut IncrementalDatabase| {
            db.upsert_description(&package, "Package: consumer\nImports: oopfixture, S7\n");
            db.set_package_metadata(vec![PackageInfo {
                root: package.clone(),
                namespace: Some(String::new()),
                dir_sources: BTreeSet::new(),
                native_routines: BTreeSet::new(),
            }]);
        };
        set_package(&mut db);
        check(&db, &path);

        let first = path.with_file_name("a.R");
        let last = path.with_file_name("z.R");
        let call = "oopfixture::measure(1L)\n";
        let registration = source.lines().nth(1).unwrap();
        let files = [
            db.upsert_file(&first, call),
            db.upsert_file(&path, registration),
            db.upsert_file(&last, call),
        ];
        db.set_workspace_members(files.to_vec(), vec![ws_root()]);
        set_package(&mut db);
        let snapshot = db.snapshot();
        assert!(implementations_at(&snapshot, &first, call, 15).is_empty());
        let locations = implementations_at(&snapshot, &last, call, 15);
        assert_eq!(locations.len(), 1, "{locations:?}");
        assert_eq!(locations[0].uri, uri::from_path(&path).unwrap());
    }

    #[test]
    fn implementation_finds_external_s4_methods() {
        let source = "methods::setClass('Example')\nmethods::setMethod('show', 'Example', function(object) object)\nmethods::show(x)\n";
        let snapshot = rename_workspace(source, "");
        let path = ws_path("a.R");
        let offset = source.find("show(x)").unwrap() + 1;
        assert_eq!(
            implementation_lines(implementations_at(&snapshot, &path, source, offset)),
            [1]
        );
    }

    #[test]
    fn implementation_finds_methods_of_indexed_s4_generics() {
        use crate::rindex::harvest::{HarvestOptions, harvest_package};
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rindex/oopfixture");
        let indexed = harvest_package(&fixture, HarvestOptions::default(), 0).unwrap();
        let package = ws_path("consumer");
        let path = package.join("R/source.R");
        let source = "methods::setClass('Example')\nmethods::setMethod('area', 'Example', function(x, scale = 1) x)\noopfixture::area(x)\narea(x)\n";
        let mut db = IncrementalDatabase::default();
        let directory = tempfile::tempdir().unwrap();
        db.set_library_index(lazy_index(&indexed, directory.path()));
        let file = db.upsert_file(&path, source);
        db.set_workspace_members(vec![file], vec![ws_root()]);
        db.refresh_descriptions([package.clone()]);
        db.upsert_description(&package, "Package: consumer\nImports: oopfixture\n");
        db.set_package_metadata(vec![PackageInfo {
            root: package,
            namespace: Some(
                "importMethodsFrom(oopfixture,area)\nimportFrom(oopfixture,area)\n".into(),
            ),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        for (offset, _) in source.match_indices("area(x)") {
            assert_eq!(
                implementation_lines(implementations_at(
                    &db.snapshot(),
                    &path,
                    source,
                    offset + 1
                )),
                [1]
            );
        }
    }

    #[test]
    fn namespace_class_export_links_in_both_directions() {
        let package = ws_path("pkg");
        let path = package.join("R/classes.R");
        let namespace = package.join("NAMESPACE");
        let source = "methods::setClass('Example')\n";
        let text = "exportClasses(Example)\n";
        let mut db = IncrementalDatabase::default();
        let file = db.upsert_file(&path, source);
        db.set_workspace_members(vec![file], vec![ws_root()]);
        db.upsert_description(&package, "Package: pkg\n");
        db.set_package_metadata(vec![PackageInfo {
            root: package,
            namespace: Some(text.into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        let snapshot = db.snapshot();
        let encoding = PositionEncoding::Utf16;
        let namespace_uri = uri::from_path(&namespace).unwrap();
        let source_uri = uri::from_path(&path).unwrap();
        let result = definition_via_db(
            &snapshot,
            &namespace,
            &namespace_uri,
            &TextBuffer::from(text),
            Position::new(0, 15),
            encoding,
            None,
        );
        let Some(GotoDefinitionResponse::Scalar(definition)) = result else {
            panic!("expected class source, got {result:?}");
        };
        assert_eq!(definition.uri, source_uri);
        assert_eq!(definition.range.start, Position::new(0, 18));
        let references = references_via_db(
            &snapshot,
            &path,
            &source_uri,
            &TextBuffer::from(source),
            Position::new(0, 20),
            false,
            encoding,
            &[],
        )
        .unwrap();
        assert_eq!(
            references,
            [Location::new(
                namespace_uri,
                Range::new(Position::new(0, 14), Position::new(0, 21))
            )]
        );
    }

    #[test]
    fn member_navigation_follows_inheritance_and_receiver_ownership() {
        let source = r#"Parent <- R6::R6Class("Parent", public = list(read = function() 1))
Child <- R6::R6Class("Child", inherit = Parent)
Other <- R6::R6Class("Other", public = list(read = function() 2))
Child$new()$read()
Other$new()$read()
"#;
        let snapshot = rename_workspace(source, "");
        let path = ws_path("a.R");
        let uri = uri::from_path(&path).unwrap();
        let buffer = TextBuffer::from(source);
        let encoding = PositionEncoding::Utf16;
        let position = Position::new(3, 13);
        let Some(GotoDefinitionResponse::Scalar(definition)) =
            definition_via_db(&snapshot, &path, &uri, &buffer, position, encoding, None)
        else {
            panic!("expected the inherited member declaration");
        };
        assert_eq!(definition.range.start, Position::new(0, 46));
        let references = references_via_db(
            &snapshot,
            &path,
            &uri,
            &buffer,
            position,
            true,
            encoding,
            &[],
        )
        .unwrap();
        assert_eq!(implementation_lines(references), [0, 3]);
    }

    #[test]
    fn generator_alias_navigation_follows_the_class_and_excludes_shadowed_names() {
        let source = r#"Builder <- R6::R6Class("Example")
Alias <- Builder
Alias$new()
function(Alias) Alias$new()
"#;
        let snapshot = rename_workspace(source, "");
        let path = ws_path("a.R");
        let uri = uri::from_path(&path).unwrap();
        let buffer = TextBuffer::from(source);
        let encoding = PositionEncoding::Utf16;
        let position = Position::new(2, 1);
        let Some(GotoDefinitionResponse::Scalar(definition)) =
            definition_via_db(&snapshot, &path, &uri, &buffer, position, encoding, None)
        else {
            panic!("expected the source class declaration");
        };
        assert_eq!(definition.range.start, Position::new(0, 23));
        let references = references_via_db(
            &snapshot,
            &path,
            &uri,
            &buffer,
            position,
            false,
            encoding,
            &[],
        )
        .unwrap();
        assert_eq!(implementation_lines(references), [1, 1, 2]);
    }

    #[test]
    fn implementation_follows_calls_to_promoted_s4_generics() {
        let source = r#"describe <- function(x) "ordinary"
methods::setMethod("describe", "numeric", function(x) 1)
methods::setMethod("describe", "character", function(x) 2)
describe(1)
alias <- describe
alias("x")
"#;
        let snapshot = rename_workspace(source, "");
        for call in ["describe(1)", "alias(\"x\")"] {
            let locations = implementations_at(
                &snapshot,
                &ws_path("a.R"),
                source,
                source.find(call).unwrap() + 1,
            );
            assert_eq!(implementation_lines(locations), [1, 2], "{call}");
        }
    }

    #[test]
    fn implementation_keeps_replaced_s4_generic_objects_separate() {
        let source = r#"methods::setGeneric("describe", function(x) standardGeneric("describe"))
old <- describe
methods::setMethod("describe", "character", function(x) "first")
methods::setGeneric("describe", function(y) standardGeneric("describe"))
methods::setMethod(old, "numeric", function(x) "old")
methods::setMethod(describe, "numeric", function(y) "new")
"#;
        let snapshot = rename_workspace(source, "");
        for ((offset, _), expected) in source
            .match_indices("setGeneric(\"describe\"")
            .zip([&[2, 4][..], &[5]])
        {
            let locations = implementations_at(
                &snapshot,
                &ws_path("a.R"),
                source,
                offset + "setGeneric(\"".len(),
            );
            assert_eq!(implementation_lines(locations), expected);
        }
        let offset = source.find("setMethod(\"describe\"").unwrap() + "setMethod(\"".len();
        let locations = implementations_at(&snapshot, &ws_path("a.R"), source, offset);
        assert_eq!(implementation_lines(locations), [2, 4]);
    }

    #[test]
    fn implementation_follows_the_body_bound_when_s4_registers() {
        let source = r#"impl <- function(x) "before"
source("b.R")
methods::setGeneric("describe", function(x) standardGeneric("describe"))
methods::setMethod("describe", "numeric", (impl))
impl <- function(x) "after"
"#;
        let snapshot = rename_workspace(source, "impl <- function(x) 'sourced'\n");
        for needle in ["\"describe\"", "setMethod"] {
            let locations = implementations_at(
                &snapshot,
                &ws_path("a.R"),
                source,
                source.find(needle).unwrap() + 1,
            );
            assert_eq!(locations.len(), 1, "{needle}: {locations:?}");
            assert_eq!(locations[0].uri, uri::from_path(&ws_path("b.R")).unwrap());
            assert_eq!(locations[0].range.start, Position::new(0, 8));
        }
    }

    #[test]
    fn implementation_withholds_an_ambiguous_deferred_s4_body() {
        let prefix = r#"methods::setGeneric("describe", function(x) standardGeneric("describe"))
impl <- function(x) "before"
"#;
        for tail in [
            r#"register <- function() methods::setMethod("describe", "numeric", impl)
register()
impl <- function(x) "after"
register()
"#,
            r#"register <- function() {
    source("b.R")
    methods::setMethod("describe", "numeric", impl)
}
"#,
        ] {
            let source = format!("{prefix}{tail}");
            let snapshot = rename_workspace(&source, "impl <- function(x) 'sourced'\n");
            for needle in ["\"describe\"", "setMethod"] {
                assert!(
                    implementations_at(
                        &snapshot,
                        &ws_path("a.R"),
                        &source,
                        source.find(needle).unwrap() + 1,
                    )
                    .is_empty(),
                    "{needle} in {source}"
                );
            }
        }
    }

    #[test]
    fn implementation_uses_the_s3_namespace_registry() {
        let source = "describe <- function(x) UseMethod('describe')\n";
        let package = ws_path("pkg");
        let path = package.join("R/a.R");
        let body_path = package.join("R/b.R");
        let mut db = IncrementalDatabase::default();
        let a = db.upsert_file(&path, source);
        let b = db.upsert_file(
            &body_path,
            "handler <- function(x) x\ndescribe_number <- handler\n",
        );
        db.set_workspace_members(vec![a, b], vec![ws_root()]);
        db.refresh_descriptions([package.clone()]);
        db.upsert_description(&package, "Package: pkg\n");
        db.set_package_metadata(vec![PackageInfo {
            root: package,
            namespace: Some("S3method(describe, numeric, describe_number)\n".into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        let locations = implementations_at(&db.snapshot(), &path, source, 1);
        assert_eq!(locations.len(), 1, "{locations:?}");
        assert_eq!(locations[0].uri, uri::from_path(&body_path).unwrap());
        assert_eq!(locations[0].range.start, Position::new(0, 11));
    }

    #[test]
    fn implementation_keeps_s3_group_registrations_off_local_generics() {
        let source = "Ops <- function(x) UseMethod('Ops')\nbase::registerS3method('Ops', 'foo', function(e1, e2) e1)\n";
        let snapshot = rename_workspace(source, "");
        let path = ws_path("a.R");
        assert!(implementations_at(&snapshot, &path, source, 1).is_empty());
        let registration = source.find("registerS3method").unwrap() + 1;
        assert_eq!(
            implementation_lines(implementations_at(&snapshot, &path, source, registration)),
            [1]
        );
        let generic = source.find("registerS3method('Ops'").unwrap() + "registerS3method('".len();
        assert_eq!(
            implementation_lines(implementations_at(&snapshot, &path, source, generic)),
            [1]
        );
    }

    #[test]
    fn implementation_confirms_the_s4_registrar() {
        let source = "source('b.R')\nmethods::setGeneric('describe', function(x) standardGeneric('describe'))\nsetMethod('describe', 'numeric', function(x) 1)\n";
        let snapshot = rename_workspace(source, "setMethod <- function(...) NULL\n");
        let offset = source.find("setMethod").unwrap() + 1;
        assert!(implementations_at(&snapshot, &ws_path("a.R"), source, offset).is_empty());
    }

    #[test]
    fn implementation_follows_class_uses_across_source_files() {
        let source =
            "source('b.R')\nChild <- R6::R6Class('Child', inherit = Parent)\nParent$new()\n";
        let snapshot = rename_workspace(source, "Parent <- R6::R6Class('Parent')\n");
        let offset = source.find("Parent$new").unwrap() + 1;
        let locations = implementations_at(&snapshot, &ws_path("a.R"), source, offset);
        assert_eq!(implementation_lines(locations), [1]);
    }

    #[test]
    fn implementation_follows_qualified_class_references() {
        let source = "pkg::Parent$new()\n";
        let package = ws_path("pkg");
        let definitions = package.join("R/a.R");
        let path = ws_path("a.R");
        let mut db = IncrementalDatabase::default();
        let a = db.upsert_file(&path, source);
        let b = db.upsert_file(
            &definitions,
            "Parent <- R6::R6Class('Parent')\nChild <- R6::R6Class('Child', inherit = Parent)\n",
        );
        db.set_workspace_members(vec![a, b], vec![ws_root()]);
        db.refresh_descriptions([package.clone()]);
        db.upsert_description(&package, "Package: pkg\n");
        db.set_package_metadata(vec![PackageInfo {
            root: package,
            namespace: Some("export(Parent)\n".into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        let snapshot = db.snapshot();
        let locations = implementations_at(&snapshot, &path, source, "pkg::P".len());
        assert_eq!(locations.len(), 1, "{locations:?}");
        assert_eq!(locations[0].uri, uri::from_path(&definitions).unwrap());
        assert_eq!(locations[0].range.start.line, 1);
        assert!(implementations_at(&snapshot, &path, source, 1).is_empty());
    }

    #[test]
    fn navigation_follows_qualified_generics_and_aliases() {
        let caller = ws_path("caller.R");
        let package = ws_path("pkg");
        let declaration = package.join("R/generic.R");
        let source = "alias <- pkg::describe\nalias(x)\nfunction(alias) alias(x)\npkg::describe.example(x)\npkg:::hidden(x)\npkg::hidden(x)\nhidden_alias <- (pkg:::hidden)\nhidden_alias(x)\npkg:::describe.hidden(x)\n";
        let mut db = IncrementalDatabase::default();
        let a = db.upsert_file(&caller, source);
        let b = db.upsert_file(
            &declaration,
            "describe <- function(x) UseMethod('describe')\ndescribe.example <- function(x) x\nhidden <- function(x) UseMethod('hidden')\ndescribe.hidden <- function(x) x\n",
        );
        db.set_workspace_members(vec![a, b], vec![ws_root()]);
        db.refresh_descriptions([package.clone()]);
        db.upsert_description(&package, "Package: pkg\n");
        db.set_package_metadata(vec![PackageInfo {
            root: package.clone(),
            namespace: Some("export(describe, describe.example)\n".into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        let snapshot = db.snapshot();
        let uri = uri::from_path(&caller).unwrap();
        let buffer = TextBuffer::from(source);
        let encoding = PositionEncoding::Utf16;
        for position in [
            Position::new(0, 1),
            Position::new(0, 15),
            Position::new(1, 1),
        ] {
            let Some(GotoDefinitionResponse::Scalar(location)) =
                definition_via_db(&snapshot, &caller, &uri, &buffer, position, encoding, None)
            else {
                panic!("expected a source generic at {position:?}");
            };
            assert_eq!(
                location.uri,
                uri::from_path(&declaration).unwrap(),
                "{position:?}"
            );
            assert_eq!(location.range.start, Position::new(0, 0));
        }
        for (line, column, expected) in [
            (3, 6, Some(1)),
            (4, 7, Some(2)),
            (5, 6, None),
            (6, 1, Some(2)),
            (7, 1, Some(2)),
            (8, 7, Some(3)),
        ] {
            match definition_via_db(
                &snapshot,
                &caller,
                &uri,
                &buffer,
                Position::new(line, column),
                encoding,
                None,
            ) {
                Some(GotoDefinitionResponse::Scalar(location)) => {
                    assert_eq!(location.uri, uri::from_path(&declaration).unwrap());
                    assert_eq!(Some(location.range.start.line), expected);
                }
                None => assert_eq!(expected, None, "qualified call on line {line}"),
                other => panic!("unexpected definition response {other:?}"),
            }
        }
        let method = source.find(":::describe.hidden").unwrap() + 4;
        let implementations = implementations_at(&snapshot, &caller, source, method);
        assert_eq!(implementations.len(), 1, "{implementations:?}");
        assert_eq!(
            implementations[0].uri,
            uri::from_path(&declaration).unwrap()
        );
        assert_eq!(implementations[0].range.start.line, 3);
        let method_references = references_via_db(
            &snapshot,
            &caller,
            &uri,
            &buffer,
            buffer.line_index().byte_to_position(method, encoding),
            false,
            encoding,
            &[],
        )
        .unwrap();
        assert_eq!(implementation_lines(method_references), [8]);

        let refs = references_via_db(
            &snapshot,
            &caller,
            &uri,
            &buffer,
            Position::new(1, 1),
            false,
            encoding,
            &[],
        )
        .unwrap();
        assert_eq!(
            implementation_lines(
                refs.iter()
                    .filter(|location| location.uri == uri)
                    .cloned()
                    .collect()
            ),
            [0, 0, 1]
        );
        assert!(refs.iter().any(|location| location.uri == uri::from_path(&package.join("NAMESPACE")).unwrap()));
    }

    #[test]
    fn implementation_follows_an_implicit_s3method_binding() {
        let source = "describe <- function(x) UseMethod('describe')\ndescribe.number <- function(x) 1\n.S3method('describe', 'number')\ndescribe.number <- function(x) 2\n";
        let snapshot = rename_workspace(source, "");
        let offset = source.find(".S3method").unwrap() + 1;
        let locations = implementations_at(&snapshot, &ws_path("a.R"), source, offset);
        assert_eq!(implementation_lines(locations), [1]);
    }

    #[test]
    fn implementation_follows_a_local_implicit_s3method_binding() {
        let source = r#"describe <- function(x) UseMethod("describe")
describe.number <- function(x) 1
register <- function() {
    describe.number <- function(x) 2
    .S3method("describe", "number")
}
"#;
        let snapshot = rename_workspace(source, "");
        let offset = source.find(".S3method").unwrap() + 1;
        let locations = implementations_at(&snapshot, &ws_path("a.R"), source, offset);
        assert_eq!(implementation_lines(locations), [3]);
    }

    #[test]
    fn implementation_resolves_nested_sources_in_their_execution_directory() {
        let source = r#"source("sub/load.R")
methods::setGeneric("describe", function(x) standardGeneric("describe"))
methods::setMethod("describe", "numeric", impl)
"#;
        let snapshot = rename_workspace_files(&[
            ("a.R", source),
            ("sub/load.R", "source('body.R')\n"),
            ("body.R", "impl <- function(x) 'expected'\n"),
            ("sub/body.R", "impl <- function(x) 'other directory'\n"),
        ]);
        let offset = source.find("setMethod").unwrap() + 1;
        let locations = implementations_at(&snapshot, &ws_path("a.R"), source, offset);
        assert_eq!(locations.len(), 1, "{locations:?}");
        assert_eq!(
            locations[0].uri,
            uri::from_path(&ws_path("body.R")).unwrap()
        );
    }

    #[test]
    fn implementation_finds_registered_method() {
        let source = "methods::setGeneric('describe', function(x) standardGeneric('describe'))\nmethods::setMethod('describe', 'numeric', function(x) x)\n";
        let snapshot = IncrementalDatabase::default().snapshot();
        let path = ws_path("classes.R");
        let uri = uri::from_path(&path).unwrap();
        let buffer = TextBuffer::from(source);
        let encoding = PositionEncoding::Utf16;
        for name in ["describe", "setMethod"] {
            let position = buffer
                .line_index()
                .byte_to_position(source.find(name).unwrap(), encoding);
            let response =
                implementation_via_db(&snapshot, &path, &uri, &buffer, position, encoding, None);
            let Some(GotoDefinitionResponse::Scalar(location)) = response else {
                panic!("expected the registered implementation for {name}, got {response:?}");
            };
            assert_eq!(location.range.start.line, 1);
        }
    }

    #[test]
    fn implementation_respects_s4_generic_package_identity() {
        let source = "methods::setGeneric('describe', function(x) standardGeneric('describe'), package = 'foo')\nmethods::setGeneric('describe', function(x) standardGeneric('describe'), package = 'bar')\nmethods::setMethod('describe', 'numeric', function(x) x)\n";
        let snapshot = IncrementalDatabase::default().snapshot();
        let path = ws_path("classes.R");
        let uri = uri::from_path(&path).unwrap();
        let buffer = TextBuffer::from(source);
        for line in [0, 1] {
            let response = implementation_via_db(
                &snapshot,
                &path,
                &uri,
                &buffer,
                Position::new(line, 23),
                PositionEncoding::Utf16,
                None,
            );
            assert!(response.is_none(), "{response:?}");
        }
    }

    #[test]
    fn implementation_follows_sourced_function() {
        let source = "source('body.R')\nmethods::setGeneric('describe', function(x) standardGeneric('describe'))\nmethods::setMethod('describe', 'numeric', impl)\n";
        let path = ws_path("classes.R");
        let body_path = path.with_file_name("body.R");
        let mut db = IncrementalDatabase::default();
        let registration = db.upsert_file(&path, source);
        let body = db.upsert_file(
            &body_path,
            "impl <- function(x) x\nalias <- impl\nimpl <- alias\n",
        );
        db.set_workspace_members(
            vec![registration, body],
            vec![path.parent().unwrap().into()],
        );
        let snapshot = db.snapshot();
        let uri = uri::from_path(&path).unwrap();
        let buffer = TextBuffer::from(source);
        for name in ["describe", "setMethod"] {
            let encoding = PositionEncoding::Utf16;
            let position = buffer
                .line_index()
                .byte_to_position(source.find(name).unwrap() + 1, encoding);
            let response =
                implementation_via_db(&snapshot, &path, &uri, &buffer, position, encoding, None);
            let Some(GotoDefinitionResponse::Scalar(location)) = response else {
                panic!("expected the sourced implementation for {name}, got {response:?}");
            };
            assert_eq!(location.uri, uri::from_path(&body_path).unwrap());
            assert_eq!(
                location.range,
                Range::new(Position::new(0, 8), Position::new(0, 21))
            );
        }
    }

    #[test]
    fn formal_class_navigation_uses_registrations_and_ignores_unrelated_strings() {
        let source = "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setGeneric('describe', function(x) standardGeneric('describe'))\nmethods::setMethod('describe', 'Parent', function(x) x)\nmethods::new('Parent')\nlabel <- 'Parent'\n";
        let snapshot = IncrementalDatabase::default().snapshot();
        let path = &ws_path("classes.R");
        let uri = uri::from_path(path).unwrap();
        let buffer = TextBuffer::from(source);
        let encoding = PositionEncoding::Utf16;
        let position = |offset| buffer.line_index().byte_to_position(offset, encoding);
        let use_offset = source.find("contains = 'Parent'").unwrap() + "contains = '".len();
        let got = definition_via_db(
            &snapshot,
            path,
            &uri,
            &buffer,
            position(use_offset),
            encoding,
            None,
        )
        .unwrap();
        let GotoDefinitionResponse::Scalar(definition) = got else {
            panic!("expected a single definition");
        };
        assert_eq!(definition.range.start.line, 0);
        let references = references_via_db(
            &snapshot,
            path,
            &uri,
            &buffer,
            position(use_offset),
            true,
            encoding,
            &[],
        )
        .unwrap();
        assert_eq!(
            references
                .iter()
                .map(|loc| loc.range.start.line)
                .collect::<Vec<_>>(),
            [0, 1, 3, 4]
        );
        let unrelated = source.rfind("'Parent'").unwrap() + 1;
        assert!(
            definition_via_db(
                &snapshot,
                path,
                &uri,
                &buffer,
                position(unrelated),
                encoding,
                None,
            )
            .is_none()
        );
    }

    #[test]
    fn formal_class_navigation_resolves_across_live_package_files() {
        let root = ws_root();
        let parent_path = root.join("R/parent.R");
        let child_path = root.join("R/child.R");
        let mut db = IncrementalDatabase::default();
        let parent = db.upsert_file(&parent_path, "methods::setClass('Parent')\n");
        let tracked = "methods::setClass('Child', contains = 'Parent')\n";
        let child = db.upsert_file(&child_path, tracked);
        db.set_workspace_members(vec![parent, child], vec![root.clone()]);
        db.refresh_descriptions([root.clone()]);
        db.upsert_description(&root, "Package: classes\n");
        db.set_package_metadata(vec![PackageInfo {
            root,
            namespace: Some(String::new()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        let snapshot = db.snapshot();
        let uri = uri::from_path(&child_path).unwrap();
        let parent_uri = uri::from_path(&parent_path).unwrap();
        let live = format!("\n\n{tracked}");
        let buffer = TextBuffer::from(live.as_str());
        let encoding = PositionEncoding::Utf16;
        let offset = live.find("'Parent'").unwrap() + 1;
        let position = buffer.line_index().byte_to_position(offset, encoding);
        let Some(GotoDefinitionResponse::Scalar(definition)) = definition_via_db(
            &snapshot,
            &child_path,
            &uri,
            &buffer,
            position,
            encoding,
            None,
        ) else {
            panic!("expected a definition in the parent file");
        };
        assert_eq!(definition.uri, parent_uri);
        let references = references_via_db(
            &snapshot,
            &child_path,
            &uri,
            &buffer,
            position,
            false,
            encoding,
            &[],
        )
        .unwrap();
        let [reference] = references.as_slice() else {
            panic!("expected only the use in the live buffer");
        };
        assert_eq!(reference.uri, uri);
        assert_eq!(reference.range.start.line, 2);
    }

    #[test]
    fn formal_class_navigation_checks_cross_file_callees() {
        let root = ws_root();
        let parent_path = root.join("R/parent.R");
        let body_path = root.join("R/body.R");
        let shadowed_path = root.join("R/shadowed.R");
        let parent_source = "methods::setClass('Parent')\nsetClass <- function(...) NULL\nsignature <- function(...) 'Other'\nc <- function(...) 'Other'\n";
        let body_source = "make <- function() methods::new('Parent')\n\
            methods::setMethod('describe', methods::signature(x = 'Parent'), function(x) x)\n";
        let shadowed_source = "setClass('Child', contains = 'Parent')\n";
        let signature_path = root.join("R/signature.R");
        let vector_path = root.join("R/vector.R");
        let signature_source =
            "methods::setMethod('describe', signature(x = 'Parent'), function(x) x)\n";
        let vector_source = "methods::setMethod('describe', c(x = 'Parent'), function(x) x)\n";
        let mut db = IncrementalDatabase::default();
        let signature = db.upsert_file(&signature_path, signature_source);
        let vector = db.upsert_file(&vector_path, vector_source);
        let parent = db.upsert_file(&parent_path, parent_source);
        let body = db.upsert_file(&body_path, body_source);
        let shadowed = db.upsert_file(&shadowed_path, shadowed_source);
        db.set_workspace_members(
            vec![parent, body, shadowed, signature, vector],
            vec![root.clone()],
        );
        db.refresh_descriptions([root.clone()]);
        db.upsert_description(&root, "Package: classes\nImports: methods\n");
        db.set_package_metadata(vec![PackageInfo {
            root,
            namespace: Some("importFrom(methods, setClass, signature)".into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        let snapshot = db.snapshot();
        let encoding = PositionEncoding::Utf16;
        let query = |path: &Path, source: &str| {
            let uri = uri::from_path(path).unwrap();
            let buffer = TextBuffer::from(source);
            let offset = source.find("'Parent'").unwrap() + 1;
            let position = buffer.line_index().byte_to_position(offset, encoding);
            (
                definition_via_db(&snapshot, path, &uri, &buffer, position, encoding, None),
                references_via_db(
                    &snapshot,
                    path,
                    &uri,
                    &buffer,
                    position,
                    false,
                    encoding,
                    &[],
                ),
            )
        };
        let body_uri = uri::from_path(&body_path).unwrap();
        // Exercise both cold and warm workspace lookups.
        for _ in 0..2 {
            let (_, references) = query(&parent_path, parent_source);
            let references = references.unwrap();
            assert_eq!(
                references
                    .iter()
                    .map(|location| (&location.uri, location.range.start.line))
                    .collect::<Vec<_>>(),
                [(&body_uri, 0), (&body_uri, 1)]
            );
        }

        assert!(query(&body_path, body_source).0.is_some());
        for (path, source) in [
            (&shadowed_path, shadowed_source),
            (&signature_path, signature_source),
            (&vector_path, vector_source),
        ] {
            let (definition, references) = query(path, source);
            assert!(definition.is_none(), "{source}");
            assert!(
                references.is_none_or(|locations| locations.is_empty()),
                "{source}"
            );
        }
    }

    #[test]
    fn generic_registration_navigation_only_links_semantic_uses() {
        let source = "methods::setGeneric('describe', function(x) standardGeneric('describe'))\nmethods::setMethod('describe', 'numeric', function(x) x)\nlabel <- 'describe'\nmethods::setMethod('describe', 'character', function(x) x, where = custom)\n";
        let snapshot = IncrementalDatabase::default().snapshot();
        let path = &ws_path("generics.R");
        let uri = uri::from_path(path).unwrap();
        let buffer = TextBuffer::from(source);
        let encoding = PositionEncoding::Utf16;
        let offset = source.find("setMethod('").unwrap() + "setMethod('".len();
        let position = buffer.line_index().byte_to_position(offset, encoding);
        let Some(GotoDefinitionResponse::Scalar(definition)) =
            definition_via_db(&snapshot, path, &uri, &buffer, position, encoding, None)
        else {
            panic!("expected the registered generic definition");
        };
        assert_eq!(definition.range.start.line, 0);
        assert_eq!(
            definition.range.start.character,
            "methods::setGeneric(".len() as u32
        );
        let references = references_via_db(
            &snapshot,
            path,
            &uri,
            &buffer,
            position,
            true,
            encoding,
            &[],
        )
        .unwrap();
        assert!(references.iter().any(|loc| loc.range.start.line == 1));
        assert!(references.iter().all(|loc| loc.range.start.line < 2));
    }

    #[test]
    fn known_receivers_never_borrow_usage_from_unknown_branches() {
        let source = "A <- R6::R6Class('A', public = list(`odd name` = 1))\nobj <- if (flag) A$new() else unknown\nobj$guessed\nobj$";
        let CompletionResponse::List(list) =
            completion::compute_completions(source, source.len(), &IndexedProvider::empty())
                .unwrap()
        else {
            panic!("expected a completion list");
        };
        assert!(!list.items.iter().any(|item| item.label == "guessed"));
        let member = list
            .items
            .iter()
            .find(|item| item.label == "odd name")
            .unwrap();
        assert_eq!(member.insert_text.as_deref(), Some("`odd name`"));
    }

    #[test]
    fn source_generics_show_formals_and_only_proven_dispatch() {
        for (source, label, selected) in [
            (
                "describe <- function(x, ...) UseMethod('describe')\ndescribe.demo <- function(x, ..., extra = 1) x\ndescribe(structure(1, class = 'demo'))",
                "describe(x, ...)",
                true,
            ),
            (
                "describe <- function(x, ...) UseMethod('describe')\ndescribe.demo <- function(x, ..., extra = 1) x\ndescribe(unknown)",
                "describe(x, ...)",
                false,
            ),
            (
                "methods::setClass('Demo', slots = c(x = 'numeric'))\nmethods::setGeneric('describe', function(x, scale = 1) standardGeneric('describe'))\nmethods::setMethod('describe', 'Demo', function(x, scale) x)\ndescribe(methods::new('Demo'))",
                "describe(x, scale = 1)",
                true,
            ),
            (
                "methods::setGeneric('describe', function(x, scale = 1) standardGeneric('describe'))\ndescribe(",
                "describe(x, scale = 1)",
                false,
            ),
            (
                "methods::setGeneric('describe', function(x) standardGeneric('describe'))\nmethods::setMethod('describe', 'missing', function(x) 0)\ndescribe(",
                "describe(x)",
                false,
            ),
        ] {
            let offset = source.rfind("describe(").unwrap() + "describe(".len();
            let help = signature::compute_signature_help(source, offset, &IndexedProvider::empty())
                .unwrap_or_else(|| panic!("no generic signature for {source}"));
            let [info] = help.signatures.as_slice() else {
                panic!("expected the generic as the only signature");
            };
            assert_eq!(info.label, label);
            assert_eq!(help.active_parameter, Some(0));
            assert_eq!(
                matches!(&info.documentation, Some(Documentation::MarkupContent(markdown))
                    if markdown.value.contains("Selected method")),
                selected,
                "{source}"
            );
        }
    }

    #[test]
    fn project_generic_signatures_respect_exports_and_imports() {
        let mut db = IncrementalDatabase::default();
        let provider = Path::new("/generic-provider/R/describe.R");
        let consumer = Path::new("/generic-consumer/R/use.R");
        let provider_file = db.upsert_file(
            provider,
            "describe <- function(x, ..., scale = 1) UseMethod('describe')\n",
        );
        let consumer_file = db.upsert_file(consumer, "describe(");
        db.set_workspace_members(
            vec![provider_file, consumer_file],
            vec!["/generic-provider".into(), "/generic-consumer".into()],
        );
        db.refresh_descriptions(["/generic-provider".into(), "/generic-consumer".into()]);
        db.upsert_description(Path::new("/generic-provider"), "Package: provider\n");
        db.upsert_description(
            Path::new("/generic-consumer"),
            "Package: consumer\nImports: provider\n",
        );
        for exported in [true, false] {
            db.set_package_metadata(
                [
                    (
                        "/generic-provider",
                        if exported { "export(describe)" } else { "" },
                    ),
                    ("/generic-consumer", "importFrom(provider, describe)"),
                ]
                .into_iter()
                .map(|(root, namespace)| PackageInfo {
                    root: root.into(),
                    namespace: Some(namespace.into()),
                    dir_sources: BTreeSet::new(),
                    native_routines: BTreeSet::new(),
                })
                .collect(),
            );
            for callee in ["describe", "provider::describe"] {
                let text = format!("{callee}(");
                let buffer = TextBuffer::new(text.clone());
                let position = buffer
                    .line_index()
                    .byte_to_position(text.len(), PositionEncoding::Utf16);
                let help = signature::signature_help_via_db(
                    &db.snapshot(),
                    consumer,
                    &buffer,
                    position,
                    PositionEncoding::Utf16,
                    None,
                );
                let expected = format!("{callee}(x, ..., scale = 1)");
                assert_eq!(
                    help.as_ref().map(|help| help.signatures[0].label.as_str()),
                    exported.then_some(expected.as_str()),
                    "{text}",
                );
            }
        }
    }

    #[test]
    fn installed_inherited_members_resolve_the_declaring_help_topic() {
        use crate::rindex::harvest::{HarvestOptions, harvest_package};
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rindex/oopfixture");
        let package = harvest_package(&fixture, HarvestOptions::default(), 0).unwrap();
        let indexed = IndexedProvider::from_indices([package]);
        let source = "obj <- oopfixture::Counter$new()\nobj$";
        let CompletionResponse::List(list) =
            completion::compute_completions(source, source.len(), &indexed).unwrap()
        else {
            panic!("expected a completion list");
        };
        assert!(!list.items.iter().any(|item| item.label == "secret"));
        let item = list
            .items
            .into_iter()
            .find(|item| item.label == "base_add")
            .unwrap();
        assert_eq!(item.kind, Some(CompletionItemKind::METHOD));
        assert!(
            completion::resolve_completion(item, &indexed)
                .documentation
                .is_some()
        );
        let call = format!("{source}base_add(");
        let help = signature::compute_signature_help(&call, call.len(), &indexed).unwrap();
        assert_eq!(help.signatures[0].label, "base_add(n = 2)");
        assert!(help.signatures[0].documentation.is_some());

        let call = "oopfixture::area(";
        let help = signature::compute_signature_help(call, call.len(), &indexed).unwrap();
        assert_eq!(help.signatures[0].label, "oopfixture::area(x, scale = 1)");
    }

    #[test]
    fn names_only_imports_can_make_installed_receiver_identity_ambiguous() {
        use crate::rindex::harvest::{HarvestOptions, harvest_package};
        use crate::rindex::remote::RemoteExports;

        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rindex/oopfixture");
        let package = harvest_package(&fixture, HarvestOptions::default(), 0).unwrap();
        let mut db = IncrementalDatabase::default();
        db.set_library_index(IndexedProvider::from_indices([package]));
        let path = Path::new("/consumer/R/use.R");
        let source = "obj <- Counter$new()\nobj$";
        let file = db.upsert_file(path, source);
        db.set_workspace_members(vec![file], vec!["/consumer".into()]);
        db.upsert_description(
            Path::new("/consumer"),
            "Package: consumer\nImports: oopfixture, other\n",
        );
        db.set_package_metadata(vec![PackageInfo {
            root: "/consumer".into(),
            namespace: Some("import(oopfixture)\nimport(other)".into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        let buffer = TextBuffer::new(source.to_string());
        let position = buffer
            .line_index()
            .byte_to_position(source.len(), PositionEncoding::Utf16);
        for ambiguous in [false, true] {
            let mut remote = RemoteExports::new();
            remote.insert_package("other", ambiguous.then(|| "Counter".into()));
            db.set_remote_exports(remote);
            let CompletionResponse::List(list) = completion::completion_via_db(
                &db.snapshot(),
                path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .unwrap() else {
                panic!("expected a completion list");
            };
            assert_eq!(
                list.items.iter().any(|item| item.label == "base_add"),
                !ambiguous
            );
        }
    }

    #[test]
    fn s4_constructor_signature_lists_inherited_slots_in_an_open_call() {
        let source = "methods::setClass('Parent', slots = c(size = 'numeric'))\nmethods::setClass('Child', contains = 'Parent', slots = c(label = 'character'))\nmethods::new('Child', ";
        let help =
            signature::compute_signature_help(source, source.len(), &IndexedProvider::empty())
                .unwrap();
        let info = &help.signatures[0];
        assert_eq!(info.label, "methods::new(Class, ...)");
        assert_eq!(help.active_parameter, Some(1));
        let Some(Documentation::MarkupContent(markdown)) = &info.documentation else {
            panic!("expected declared slot documentation");
        };
        assert!(markdown.value.contains("size"));
        assert!(markdown.value.contains("label"));
    }

    #[test]
    fn canceled_read_keeps_the_description_floor() {
        use std::cell::Cell;

        let package = Path::new("/s7-cancel");
        let path = package.join("R/point.R");
        let root = parse("S7::`:=`(Point, S7::new_class())").cst;
        let mut db = IncrementalDatabase::default();
        db.refresh_descriptions([package.into()]);
        db.upsert_description(package, "Package: shapes\nImports: S7 (>= 0.2.2.9000)\n");
        let floor =
            crate::project::package_facts_for(&db, &path).and_then(|facts| facts.compat.s7.clone());
        let context = ReadContext {
            snapshot: &db.snapshot(),
            path: &path,
            s7_floor: floor.as_ref(),
        };
        let first = Cell::new(true);
        let bound = with_live_model(&root, Some(context), |_, model, _| {
            if first.replace(false) {
                std::panic::resume_unwind(Box::new(salsa::Cancelled::PendingWrite));
            }
            model
                .bindings()
                .iter()
                .any(|binding| binding.name == "Point")
        });
        assert!(bound);
    }

    #[test]
    fn s7_bare_completion_uses_confirmed_named_bindings() {
        let path = Path::new("/s7-bare.R");
        let source = "library(S7)\nPoint := new_class()\nPoi";
        let buffer = TextBuffer::new(source.to_string());
        let position = buffer
            .line_index()
            .byte_to_position(source.len(), PositionEncoding::Utf16);
        let mut db = IncrementalDatabase::default();
        let file = db.upsert_file(path, buffer.text_arc());
        let floor = CompatVersion::parse("0.2.2.9000");
        db.set_s7_floor(file, floor.clone());
        let CompletionResponse::List(items) = completion::completion_via_db(
            &db.snapshot(),
            path,
            &buffer,
            position,
            PositionEncoding::Utf16,
            floor.as_ref(),
        )
        .unwrap() else {
            panic!("completion list");
        };
        assert!(items.items.iter().any(|item| item.label == "Point"));
    }

    #[test]
    fn s7_dev_names_and_properties_follow_live_buffers() {
        let path = Path::new("/s7-live.R");
        let source = "library(S7)\nPoint := S7::new_class(properties = list(x = S7::class_integer))\nobj <- Point()\nobj@";
        let mut db = IncrementalDatabase::default();
        let file = db.upsert_file(path, source);
        let floor = CompatVersion::parse("0.2.2.9000");
        db.set_s7_floor(file, floor.clone());
        for (text, expected) in [
            (source.to_string(), "x"),
            (source.replace("x =", "label ="), "label"),
        ] {
            let buffer = TextBuffer::new(text.clone());
            let position = buffer
                .line_index()
                .byte_to_position(text.len(), PositionEncoding::Utf16);
            let snapshot = db.snapshot();
            let got = completion::completion_via_db(
                &snapshot,
                path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                floor.as_ref(),
            )
            .unwrap();
            let CompletionResponse::List(items) = got else {
                panic!("completion list")
            };
            assert!(
                items.items.iter().any(|item| item.label == expected
                    && item.kind == Some(CompletionItemKind::PROPERTY))
            );
            read_jobs::with_document_semantics(
                &snapshot,
                path,
                &buffer,
                floor.as_ref(),
                |_, model| {
                    assert!(
                        model
                            .bindings()
                            .iter()
                            .any(|binding| binding.name == "Point")
                    );
                },
            );
        }
    }

    #[test]
    fn s4_slot_completion_uses_imports_and_live_buffer_class_facts() {
        let mut db = IncrementalDatabase::default();
        let parent_path = Path::new("/s4-parent/R/classes.R");
        let child_path = Path::new("/s4-child/R/classes.R");
        let parent = db.upsert_file(
            parent_path,
            "methods::setClass('Inner', slots = c(hidden = 'numeric'))\nMaker <- methods::setClass('Display', slots = c(value = 'numeric', inner = 'Inner'))\n",
        );
        let source = "Child <- methods::setClass('Child', contains = 'Display', slots = c(own = 'logical'))\nobj <- Child()\nobj@";
        let child = db.upsert_file(child_path, source);
        db.set_workspace_members(
            vec![parent, child],
            vec!["/s4-parent".into(), "/s4-child".into()],
        );
        db.refresh_descriptions(["/s4-parent".into(), "/s4-child".into()]);
        db.upsert_description(Path::new("/s4-parent"), "Package: parent\n");
        db.upsert_description(Path::new("/s4-child"), "Package: child\nImports: parent\n");
        let metadata = |exported| {
            [
                (
                    "/s4-parent",
                    if exported {
                        "exportClasses(Display)\nexport(Maker)\n"
                    } else {
                        ""
                    },
                ),
                (
                    "/s4-child",
                    "importClassesFrom(parent, Display)\nimportFrom(parent, Maker)\n",
                ),
            ]
            .into_iter()
            .map(|(root, namespace)| PackageInfo {
                root: root.into(),
                namespace: Some(namespace.into()),
                dir_sources: BTreeSet::new(),
                native_routines: BTreeSet::new(),
            })
            .collect()
        };
        db.set_package_metadata(metadata(true));
        for (text, expected) in [
            (source.to_string(), vec!["inner", "own", "value"]),
            (
                source.replace("own = 'logical'", "edited = 'character'"),
                vec!["edited", "inner", "value"],
            ),
            ("parent::Maker()@".to_string(), vec!["inner", "value"]),
            (
                "methods::new('Display')@".to_string(),
                vec!["inner", "value"],
            ),
            ("methods::new('Display')@inner@".to_string(), vec!["hidden"]),
            ("methods::new('Inner')@".to_string(), vec![]),
        ] {
            let buffer = TextBuffer::new(text.clone());
            let position = buffer
                .line_index()
                .byte_to_position(text.len(), PositionEncoding::Utf16);
            let CompletionResponse::List(list) = completion::completion_via_db(
                &db.snapshot(),
                child_path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .unwrap() else {
                panic!("completion list");
            };
            assert!(!list.is_incomplete, "{text}: {list:?}");
            assert_eq!(
                list.items
                    .iter()
                    .map(|item| item.label.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{text}"
            );
        }
        db.set_package_metadata(metadata(false));
        let text = "methods::new('Display')@";
        let buffer = TextBuffer::new(text.to_string());
        let position = buffer
            .line_index()
            .byte_to_position(text.len(), PositionEncoding::Utf16);
        let CompletionResponse::List(list) = completion::completion_via_db(
            &db.snapshot(),
            child_path,
            &buffer,
            position,
            PositionEncoding::Utf16,
            None,
        )
        .unwrap() else {
            panic!("completion list");
        };
        assert!(list.items.is_empty());
    }
    #[test]
    fn s4_completion_recovers_named_method_frames_from_live_buffers() {
        let package = Path::new("/s4-method-frame");
        let path = package.join("R/implementation.R");
        let source = "impl <- function(x) x@first\nalias <- impl\nother <- function(x) x@first\n";
        let mut db = IncrementalDatabase::default();
        let implementation = db.upsert_file(&path, source);
        let registration = db.upsert_file(
            &package.join("R/methods.R"),
            "methods::setClass('A', slots = c(first = 'numeric', second = 'numeric'))\n\
             methods::setGeneric('inspect', function(x) standardGeneric('inspect'))\n\
             methods::setMethod('inspect', 'A', alias)\n",
        );
        db.set_workspace_members(vec![implementation, registration], vec![package.into()]);
        db.set_package_metadata(vec![PackageInfo {
            root: package.into(),
            namespace: None,
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        let edited = source.replace(
            "impl <- function(x) x@first",
            "impl <- (function(x) x@first)",
        );
        for text in [
            source.to_string(),
            format!("# A live edit shifts the callable.\n{edited}"),
        ] {
            let buffer = TextBuffer::new(text);
            for ((offset, _), registered) in buffer.text().match_indices('@').zip([true, false]) {
                let position = buffer
                    .line_index()
                    .byte_to_position(offset + 1, PositionEncoding::Utf16);
                let CompletionResponse::List(list) = completion::completion_via_db(
                    &db.snapshot(),
                    &path,
                    &buffer,
                    position,
                    PositionEncoding::Utf16,
                    None,
                )
                .unwrap() else {
                    panic!("completion list");
                };
                assert_eq!(
                    list.items.iter().any(|item| item.label == "second"),
                    registered
                );
            }
        }
    }

    #[test]
    fn generator_mutations_use_live_signatures_and_keep_older_instances() {
        let mut db = IncrementalDatabase::default();
        let definitions = db.upsert_file(Path::new("/mutation-live/R/a.R"), "G <- R6::R6Class(public = list(read = function(original) original))\nRef <- methods::setRefClass('LiveMutation', methods = list(read = function(original) original))\n");
        let path = Path::new("/mutation-live/R/b.R");
        let mutations = db.upsert_file(path, "G$set('public', 'read', function(stored) stored, overwrite = TRUE)\nRef$methods(read = function(stored) stored)\n");
        db.set_workspace_members(vec![definitions, mutations], vec!["/mutation-live".into()]);
        db.set_package_metadata(vec![PackageInfo {
            root: "/mutation-live".into(),
            namespace: None,
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        for (text, expected) in [
            (
                "G$set('public', 'read', function(live = 2) live, overwrite = TRUE)\nG$new()$read()",
                "read(live = 2)",
            ),
            (
                "old <- G$new()\nG$set('public', 'read', function(live) live, overwrite = TRUE)\nold$read()",
                "read(original)",
            ),
            (
                "Ref$methods(read = function(live = 3) live)\nRef()$read()",
                "read(live = 3)",
            ),
        ] {
            let buffer = TextBuffer::new(text);
            let position = buffer
                .line_index()
                .byte_to_position(text.len() - 1, PositionEncoding::Utf16);
            let help = signature::signature_help_via_db(
                &db.snapshot(),
                path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .unwrap();
            assert_eq!(help.signatures[0].label, expected);
        }
    }

    #[test]
    fn reference_reads_share_imported_class_identity_and_live_buffer_overlays() {
        let mut db = IncrementalDatabase::default();
        let parent_path = Path::new("/ref-parent/R/parent.R");
        let child_path = Path::new("/ref-child/R/child.R");
        let parent = db.upsert_file(parent_path, "Original <- methods::setRefClass('Display', fields = list(value = 'numeric'), methods = list(initialize = function(value = 3) initFields(value = value), read = function(scale = 2) value * scale))\nAlias <- Original\n");
        let source =
            "Child <- methods::setRefClass('Child', contains = 'Display')\nobj <- Child()\nobj$r";
        let child = db.upsert_file(child_path, source);
        db.set_workspace_members(
            vec![parent, child],
            vec!["/ref-parent".into(), "/ref-child".into()],
        );
        db.refresh_descriptions(["/ref-parent".into(), "/ref-child".into()]);
        db.upsert_description(Path::new("/ref-parent"), "Package: parent\n");
        db.upsert_description(Path::new("/ref-child"), "Package: child\nImports: parent\n");
        let metadata = |exported| {
            [
                (
                    "/ref-parent",
                    if exported {
                        "exportClasses(Display)\nexport(Alias)\n"
                    } else {
                        ""
                    },
                ),
                (
                    "/ref-child",
                    "importClassesFrom(parent, Display)\nimportFrom(parent, Alias)\n",
                ),
            ]
            .into_iter()
            .map(|(root, namespace)| PackageInfo {
                root: root.into(),
                namespace: Some(namespace.into()),
                dir_sources: BTreeSet::new(),
                native_routines: BTreeSet::new(),
            })
            .collect()
        };
        db.set_package_metadata(metadata(true));
        for (text, expected) in [
            (source.to_string(), "read"),
            (source.replace("obj$r", "obj$v"), "value"),
        ] {
            let buffer = TextBuffer::new(text.clone());
            let position = buffer
                .line_index()
                .byte_to_position(text.len(), PositionEncoding::Utf16);
            let CompletionResponse::List(items) = completion::completion_via_db(
                &db.snapshot(),
                child_path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .unwrap() else {
                panic!("completion list")
            };
            assert!(
                items.items.iter().any(|item| item.label == expected),
                "{items:?}"
            );
        }
        for (text, expected) in [
            (
                "Child <- methods::setRefClass('Child', contains = 'Display')\nChild$new()",
                "new(value = 3)",
            ),
            (
                "Child <- methods::setRefClass('Child', contains = 'Display', methods = list(initialize = function(label = 'live') initFields()))\nChild$new()",
                "new(label = 'live')",
            ),
            (
                "getRefClass('Display')()",
                "getRefClass('Display')(value = 3)",
            ),
            ("parent::Alias()", "parent::Alias(value = 3)"),
            (
                "Child <- methods::setRefClass('Child', contains = 'Display', methods = list(read = function(scale = 6) callSuper()))",
                "callSuper(scale = 2)",
            ),
        ] {
            let buffer = TextBuffer::new(text.to_string());
            let offset = if text.contains("callSuper()") {
                text.find("callSuper()").unwrap() + "callSuper(".len()
            } else {
                text.len() - 1
            };
            let position = buffer
                .line_index()
                .byte_to_position(offset, PositionEncoding::Utf16);
            let help = signature::signature_help_via_db(
                &db.snapshot(),
                child_path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .unwrap_or_else(|| panic!("{text}"));
            assert_eq!(help.signatures[0].label, expected);
        }
        db.set_package_metadata(metadata(false));
        let text = "Child <- methods::setRefClass('Child', contains = 'Display')\nChild$new()";
        let buffer = TextBuffer::new(text.to_string());
        let position = buffer
            .line_index()
            .byte_to_position(text.len() - 1, PositionEncoding::Utf16);
        assert!(
            signature::signature_help_via_db(
                &db.snapshot(),
                child_path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .is_none()
        );
    }

    #[test]
    fn r6_reads_resolve_qualified_parent_aliases_and_overlay_live_buffers() {
        let mut db = IncrementalDatabase::default();
        let parent_path = Path::new("/r6-parent/R/parent.R");
        let child_path = Path::new("/r6-child/R/child.R");
        let parent = db.upsert_file(parent_path, "Original <- R6::R6Class('Display', public = list(value = 1, initialize = function(value = 3) self$value <- value, read = function(scale = 2) self$value * scale))\nAlias <- Original\n");
        let source = "Child <- R6::R6Class(inherit = parent::Alias)\nobj <- Child$new()\nobj$r";
        let child = db.upsert_file(child_path, source);
        db.set_workspace_members(
            vec![parent, child],
            vec!["/r6-parent".into(), "/r6-child".into()],
        );
        db.refresh_descriptions(["/r6-parent".into(), "/r6-child".into()]);
        db.upsert_description(Path::new("/r6-parent"), "Package: parent\n");
        db.upsert_description(Path::new("/r6-child"), "Package: child\nImports: parent\n");
        db.set_package_metadata(
            [("/r6-parent", "export(Alias)\n"), ("/r6-child", "")]
                .into_iter()
                .map(|(root, namespace)| PackageInfo {
                    root: root.into(),
                    namespace: Some(namespace.into()),
                    dir_sources: BTreeSet::new(),
                    native_routines: BTreeSet::new(),
                })
                .collect(),
        );
        // Cached syntax and a live buffer overlay must use the same parent identity.
        for (text, expected) in [
            (source.to_string(), "read"),
            (source.replace("obj$r", "obj$v"), "value"),
        ] {
            let buffer = TextBuffer::new(text.clone());
            let position = buffer
                .line_index()
                .byte_to_position(text.len(), PositionEncoding::Utf16);
            let result = completion::completion_via_db(
                &db.snapshot(),
                child_path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .unwrap();
            let CompletionResponse::List(items) = result else {
                panic!("completion list");
            };
            assert!(
                items.items.iter().any(|item| item.label == expected),
                "{items:?}"
            );
        }
        for (declaration, expected) in [
            (
                "Child <- R6::R6Class(inherit = parent::Alias)",
                "new(value = 3)",
            ),
            (
                "Child <- R6::R6Class(inherit = parent::Alias, public = list(initialize = function(label = 'live') label))",
                "new(label = 'live')",
            ),
        ] {
            let text = format!("{declaration}\nChild$new()");
            let buffer = TextBuffer::new(text.clone());
            let position = buffer
                .line_index()
                .byte_to_position(text.len() - 1, PositionEncoding::Utf16);
            let help = signature::signature_help_via_db(
                &db.snapshot(),
                child_path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .unwrap();
            assert_eq!(help.signatures[0].label, expected);
        }
        // Removing the export invalidates the same lookup instead of matching Display.
        db.set_package_metadata(
            [("/r6-parent", ""), ("/r6-child", "")]
                .into_iter()
                .map(|(root, namespace)| PackageInfo {
                    root: root.into(),
                    namespace: Some(namespace.into()),
                    dir_sources: BTreeSet::new(),
                    native_routines: BTreeSet::new(),
                })
                .collect(),
        );
        let text = "Child <- R6::R6Class(inherit = parent::Alias)\nChild$new()";
        let buffer = TextBuffer::new(text.to_string());
        let position = buffer
            .line_index()
            .byte_to_position(text.len() - 1, PositionEncoding::Utf16);
        assert!(
            signature::signature_help_via_db(
                &db.snapshot(),
                child_path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .is_none()
        );
    }
}
