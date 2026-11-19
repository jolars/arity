use std::path::Path;

use arity::incremental::IncrementalDatabase;
use arity::project::classes::ClassName;

#[test]
fn s7_signature_help_exposes_inherited_writable_properties() {
    let source = r#"
Parent <- S7::new_class("Parent", properties = list(id = S7::class_integer))
Child <- S7::new_class("Child", parent = Parent, properties = list(
    label = S7::new_property(S7::class_character, default = "child"),
    readonly = S7::new_property(getter = function(self) self@id)
))
Child()
"#;
    let help =
        arity::lsp::compute_signature_help(source, source.rfind(')').unwrap(), &Default::default())
            .unwrap();
    assert_eq!(
        help.signatures[0].label,
        "Child(id = integer(0), label = \"child\")"
    );
}

#[test]
fn named_s7_bindings_require_a_development_floor() {
    let mut db = IncrementalDatabase::default();
    let package = Path::new("/s7-names");
    let file = db.upsert_file(
        &package.join("R/classes.R"),
        "S7::`:=`(Point, S7::new_class())\nPoint()\nS7::\":=\"(Circle, S7::new_class())\n",
    );
    assert!(db.semantic_model(file).bindings().is_empty());
    db.refresh_descriptions([package.to_path_buf()]);
    for (floor, bound) in [("0.2.2", false), ("0.2.2.9000", true)] {
        db.upsert_description(
            package,
            format!("Package: shapes\nImports: S7 (>= {floor})\n"),
        );
        let model = db.semantic_model(file);
        assert_eq!(
            model
                .bindings()
                .iter()
                .any(|binding| binding.name == "Point"),
            bound
        );
        assert_eq!(
            model
                .bindings()
                .iter()
                .any(|binding| binding.name == "Circle"),
            bound
        );
        if bound {
            let facts = arity::project::classes::file_class_defs(
                model,
                &arity::incremental::parsed_tree_root(&db, file),
            );
            assert_eq!(facts.declarations[0].name, ClassName::Known("Point".into()));
        }
    }
}

#[test]
fn naming_respects_attachment_namespace_and_data_masks() {
    use arity::config::CompatVersion;
    use arity::project::PackageInfo;

    let mut db = IncrementalDatabase::default();
    let package = Path::new("/s7-mask");
    let path = package.join("R/classes.R");
    let metadata = |namespace: &str| PackageInfo {
        root: package.to_path_buf(),
        namespace: Some(namespace.to_string()),
        dir_sources: Default::default(),
        native_routines: Default::default(),
    };
    let file = db.upsert_file(
        &path,
        r#"
library(S7)
library(rlang)
library(data.table)
Point := S7::new_class()
rlang::list2(injected := factory())
rlang::pairlist2(pair_key := list())
tibble::tibble(col := rep(1L, 2L))
rlang::exec(list, exec_key := list())
rlang::exec(ExecClass := S7::new_class())
dt <- data.table::data.table(x = 1)
dt[, column := factory()]
local <- function() {
    `:=` <- function(lhs, rhs) NULL
    Shadowed := S7::new_class()
}
"#,
    );
    db.set_s7_floor(file, CompatVersion::parse("0.2.2.9000"));
    db.set_workspace_members(vec![file], vec![package.to_path_buf()]);
    for (namespace, binds_point) in [
        ("", true),
        ("import(S7)\nimportFrom(rlang, `:=`)\n", false),
        ("importFrom(rlang, `:=`)\nimport(S7)\n", true),
        ("import(S7)\nimport(rlang, except = \":=\")\n", true),
    ] {
        db.set_package_metadata(vec![metadata(namespace)]);
        let model = db.semantic_model(file);
        assert_eq!(
            model
                .bindings()
                .iter()
                .any(|binding| binding.name == "Point"),
            binds_point,
            "{namespace}"
        );
        assert_eq!(
            model
                .bindings()
                .iter()
                .any(|binding| binding.name == "ExecClass"),
            binds_point,
            "{namespace}"
        );
        for name in [
            "injected", "pair_key", "col", "exec_key", "column", "Shadowed",
        ] {
            assert!(
                !model.bindings().iter().any(|binding| binding.name == name),
                "{name}: {namespace}"
            );
        }
    }
}

#[test]
fn development_binding_matches_call_and_attachment_arguments() {
    for (source, bound) in [
        (r#"S7::`:=`(rhs = Point, lhs = S7::new_class())"#, true),
        (
            r#"library(S7); rlang::call2("f", Point := S7::new_class())"#,
            false,
        ),
        (
            r#"library(S7); rlang::call2(Point := S7::new_class())"#,
            true,
        ),
        (
            r#"library(S7, exclude = ":="); Point := S7::new_class()"#,
            false,
        ),
        (
            r#"require(S7, include.only = "new_class"); Point := S7::new_class()"#,
            false,
        ),
        (
            r#"library(S7, include.only = c("new_class", ":=")); Point := S7::new_class()"#,
            true,
        ),
        (
            r#"base::library(S7, exclude = base::c("new_class", ":=")); Point := S7::new_class()"#,
            false,
        ),
        (
            r#"library(S7, exclude = ":="); library(S7); Point := S7::new_class()"#,
            false,
        ),
        (
            r#"library(S7, exclude = filters); Point := S7::new_class()"#,
            false,
        ),
    ] {
        at_calls(source, Some("0.2.2.9000"), |_, _, model, _| {
            assert_eq!(
                model
                    .bindings()
                    .iter()
                    .any(|binding| binding.name == "Point"),
                bound,
                "{source}",
            );
        });
    }
}

fn at_calls(
    source: &str,
    version: Option<&str>,
    check: impl FnOnce(
        &arity::project::classes::ClassIndex,
        &arity::syntax::SyntaxNode,
        &arity::semantic::SemanticModel,
        &Path,
    ),
) {
    use arity::incremental::parsed_tree_root;
    use arity::project::classes::file_class_defs;
    let mut db = IncrementalDatabase::default();
    let path = Path::new("/s7-probe.R");
    let file = db.upsert_file(path, source);
    db.set_s7_floor(file, version.and_then(arity::config::CompatVersion::parse));
    let root = parsed_tree_root(&db, file);
    let model = db.semantic_model(file);
    let facts = file_class_defs(model, &root);
    let index = db.snapshot().class_index_in(path, &facts, &root, model);
    check(&index, &root, model, path);
}

#[test]
fn dispatch_ranks_left_to_right_and_respects_super_unions_and_missing() {
    use arity::ast::{AstNode, CallExpr};
    use arity::project::classes::MethodImplementationSource;
    let source = r#"
Parent <- S7::new_class("Parent")
Child <- S7::new_class("Child", parent = Parent)
g <- S7::new_generic("g", c("x", "y"))
left <- function(x, y) "left"
numeric <- function(x, y) "numeric"
missing <- function(x, y) "missing"
specific <- function(x, y) "specific"
S7::method(g, list(Parent, S7::class_any)) <- left
S7::method(g, list(S7::class_any, S7::class_integer | S7::class_double)) <- numeric
S7::method(g, list(S7::class_missing, S7::class_any)) <- missing
S7::method(g, list(Child, S7::class_any)) <- specific
g(Child(), 1L)
g(S7::super(Child(), Parent), 1L)
g(1L, 2L)
g(y = 2L)
if (flag) S7::method(g, list(Child, S7::class_any)) <- left
g(Child(), 1L)
"#;
    at_calls(source, None, |index, root, model, path| {
        let calls = root
            .descendants()
            .filter_map(CallExpr::cast)
            .filter(|call| call.callee_name().as_deref() == Some("g"))
            .collect::<Vec<_>>();
        for (call, expected) in calls.iter().zip([
            Some("specific"),
            Some("left"),
            Some("numeric"),
            Some("missing"),
            None,
        ]) {
            let result = index.s7_dispatch_at(path, root, model, call).unwrap();
            let implementation = result
                .selected
                .and_then(|method| index.s7_registration(&method)?.implementation);
            assert_eq!(
                implementation,
                expected.map(|name| MethodImplementationSource::Binding {
                    owner: Default::default(),
                    name: name.to_string()
                }),
                "{call:?}"
            );
            assert_eq!(result.complete, expected.is_some());
        }
    });
}

#[test]
fn property_reads_writes_and_constructor_context_share_one_class() {
    use arity::ast::{AstNode, CallExpr};
    use arity::project::classes::{AccessorArgument, MemberEffect};
    use arity::syntax::SyntaxElement;
    let source = r#"
Parent <- S7::new_class("Parent", package = "shapes", properties = list(x = S7::class_integer))
Child <- S7::new_class("Child", parent = Parent, properties = list(
    S7::new_property(S7::class_integer, name = "read", getter = function(self) self@x),
    value = S7::new_property(S7::class_integer, setter = function(self, value) self)
), constructor = function(x = 1L) S7::new_object(Parent(x)))
obj <- Child()
S7::prop(obj, "read")
S7::prop(obj, "value") <- 2L
S7::props(obj) <- list(value = 3L, x = 4L)
"#;
    at_calls(source, None, |index, root, model, path| {
        let mut uses = Vec::new();
        for call in root.descendants().filter_map(CallExpr::cast) {
            if call.callee_name().as_deref() == Some("new_object") {
                let class = index.s7_new_object_class(path, root, model, &call).unwrap();
                assert_eq!(index.s7_class_tag(&class).as_deref(), Some("Child"));
            }
            let value = SyntaxElement::Node(call.syntax().clone());
            if let Some(property) = index.s7_property_use(path, root, model, &value) {
                uses.push(property);
            }
        }
        assert_eq!(uses.len(), 3);
        assert_eq!(uses[0].names, ["read"]);
        let getter = index.member_access(&uses[0].receiver, "read").unwrap();
        assert!(matches!(
            getter.read,
            MemberEffect::InvokeAccessor {
                implementation: Some(_),
                argument: AccessorArgument::Missing
            }
        ));
        assert_eq!(getter.write, MemberEffect::ReadOnly);
        assert!(uses[1].write);
        assert_eq!(uses[1].names, ["value"]);
        assert!(matches!(
            index
                .member_access(&uses[1].receiver, "value")
                .unwrap()
                .write,
            MemberEffect::InvokeAccessor {
                implementation: Some(_),
                argument: AccessorArgument::AssignedValue
            }
        ));
        assert!(uses[2].write);
        assert_eq!(uses[2].names, ["value", "x"]);
    });
}

#[test]
fn explicit_s4_bridge_respects_the_version_and_exposes_only_stored_properties() {
    let source = r#"
Plain <- methods::setClass("Plain", slots = c(seed = "integer"))
Base <- S7::new_class("Base", properties = list(id = S7::class_integer))
S7::S4_register(Base)
Wrapped <- methods::setClass("Wrapped", contains = S7::S4_contains(Base), slots = c(label = "character"))
Colored <- S7::new_class("Colored", parent = methods::getClass("Plain"), properties = list(color = S7::class_character))
"#;
    for (floor, development) in [(None, false), (Some("0.2.2.9000"), true)] {
        at_calls(source, floor, |index, _, _, _| {
            let id = |name: &str| {
                index
                    .declarations
                    .iter()
                    .find(|(_, class)| class.declaration.display_name() == name)
                    .map(|(id, _)| id)
                    .unwrap()
            };
            assert!(index.s7_registered_with_s4(id("Base")));
            assert_eq!(
                index
                    .s4_slots(id("Wrapped"))
                    .unwrap()
                    .entries
                    .contains_key("id"),
                development
            );
            assert_eq!(
                index
                    .s7_api(id("Colored"))
                    .unwrap()
                    .properties
                    .iter()
                    .any(|prop| prop.name == "seed"),
                development
            );
        });
    }
}

#[test]
fn outside_packages_the_config_floor_controls_cli_def_use() {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("arity.toml");
    for (floor, succeeds) in [("0.2.2.9000", true), ("0.2.2", false)] {
        std::fs::write(&config, format!("[compat]\ns7 = \"{floor}\"\n")).unwrap();
        let mut process = Command::new(env!("CARGO_BIN_EXE_arity"))
            .current_dir(dir.path())
            .args(["lint", "--select", "undefined-symbol", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        process
            .stdin
            .take()
            .unwrap()
            .write_all(b"library(S7)\nPoint := S7::new_class()\nPoint()\n")
            .unwrap();
        let output = process.wait_with_output().unwrap();
        assert_eq!(
            output.status.success(),
            succeeds,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if !succeeds {
            assert!(String::from_utf8_lossy(&output.stderr).contains("Point"));
        }
    }
}

#[test]
fn development_exports_resolve_in_cli_file_and_stdin_lint() {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let dir = tempfile::tempdir().unwrap();
    let source = "library(S7)\nA <- new_class('A')\nS4_register(A)\nmethods::setClass('B', contains = S4_contains(A))\nS7_class_desc(A)\n";
    std::fs::write(dir.path().join("classes.R"), source).unwrap();
    for (floor, succeeds) in [("0.2.2.9000", true), ("0.2.2", false)] {
        std::fs::write(
            dir.path().join("arity.toml"),
            format!("[compat]\ns7 = '{floor}'\n"),
        )
        .unwrap();
        for input in ["classes.R", "-"] {
            let mut process = Command::new(env!("CARGO_BIN_EXE_arity"))
                .current_dir(dir.path())
                .args(["lint", "--select", "undefined-symbol", input])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            if input == "-" {
                process
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(source.as_bytes())
                    .unwrap();
            }
            let output = process.wait_with_output().unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(
                output.status.success(),
                succeeds,
                "{floor}, {input}: {stderr}"
            );
            if !succeeds {
                assert!(stderr.contains("S4_contains"), "{stderr}");
            }
        }
    }
}

#[test]
fn scope_only_package_sources_use_the_cli_s7_floor() {
    use std::process::Command;

    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path();
    std::fs::create_dir(pkg.join("R")).unwrap();
    std::fs::write(pkg.join("DESCRIPTION"), "Package: shapes\nVersion: 1.0.0\nTitle: Shapes\nDescription: Test shapes.\nLicense: MIT\nImports: S7\n").unwrap();
    std::fs::write(pkg.join("NAMESPACE"), "import(S7)\n").unwrap();
    std::fs::write(pkg.join("R/classes.R"), "Point := S7::new_class()\n").unwrap();
    std::fs::write(pkg.join("R/use.R"), "Point()\n").unwrap();
    let floor = "[compat]\ns7 = '0.2.2.9000'\n";
    std::fs::write(pkg.join("arity.toml"), floor).unwrap();
    std::fs::write(pkg.join("explicit.toml"), floor).unwrap();
    for (args, succeeds) in [
        (&["R/use.R"][..], true),
        (&["--no-config", "R/use.R"], false),
        (&["--config", "explicit.toml", "R/use.R"], true),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_arity"))
            .current_dir(pkg)
            .args(["lint", "--select", "undefined-symbol"])
            .args(args)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.success(), succeeds, "{args:?}: {stderr}");
        if !succeeds {
            assert!(stderr.contains("Point"), "{stderr}");
        }
    }
}

#[test]
fn s7_named_bindings_refresh_when_the_first_library_index_arrives() {
    use arity::config::CompatVersion;
    use arity::rindex::{provider::IndexedProvider, schema::PackageIndex};

    let mut db = IncrementalDatabase::default();
    let file = db.add_file("library(S7)\nlibrary(otherpkg)\nPoint := S7::new_class()\nPoint()\n");
    db.set_s7_floor(file, CompatVersion::parse("0.2.2.9000"));
    let has_point = |db: &IncrementalDatabase| {
        db.semantic_model(file)
            .bindings()
            .iter()
            .any(|binding| binding.name == "Point")
    };
    assert!(!has_point(&db));
    db.set_library_index(IndexedProvider::from_indices([PackageIndex {
        objects: Default::default(),
        schema_version: arity::rindex::schema::SCHEMA_VERSION,
        package: "otherpkg".into(),
        version: "1.0.0".into(),
        title: None,
        r_version: None,
        lib_path: String::new(),
        harvested_at: 0,
        attaches: Vec::new(),
        symbols: Vec::new(),
    }]));
    assert!(has_point(&db));
}

#[test]
fn reusable_s7_properties_keep_confirmed_binding_names() {
    let source = r#"
library(S7)
radius := new_property(class_double)
alias <- radius
width <- new_property(class_integer)
Circle := new_class(properties = list(alias, diameter = radius))
Unnamed := new_class(properties = list(width))
"#;
    at_calls(source, Some("0.2.2.9000"), |index, _, _, _| {
        let api = |name| {
            let (id, _) = index
                .declarations
                .iter()
                .find(|(_, class)| class.declaration.display_name() == name)
                .unwrap();
            index.s7_api(id).unwrap()
        };
        let circle = api("Circle");
        assert!(circle.complete);
        assert_eq!(
            circle
                .properties
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["radius", "diameter"]
        );
        assert!(circle.constructor.is_some());
        assert!(!api("Unnamed").complete);
    });
}

#[test]
fn s7_constructor_keeps_effective_s4_slot_order() {
    let source = r#"
Base <- methods::setClass("Base", contains = "numeric", slots = c(z = "character", a = "integer"))
Middle <- methods::setClass("Middle", contains = "Base", slots = c(v = "logical", b = "raw"))
Derived <- S7::new_class("Derived", parent = methods::getClass("Middle"), properties = list(c = S7::class_double))
"#;
    assert_constructor_parameters(
        source,
        Some("0.2.2.9000"),
        "Derived",
        &[
            (".Data", "integer(0)"),
            ("v", "logical(0)"),
            ("b", "raw(0)"),
            ("z", "character(0)"),
            ("a", "integer(0)"),
            ("c", "numeric(0)"),
        ],
    );
}

#[test]
fn s7_default_constructors_forward_only_between_distinct_packages() {
    for (parent, child, forwards) in [
        ("'shapes'", "NULL", false),
        ("NULL", "'shapes'", false),
        ("'shapes'", "'shapes'", false),
        ("'shapes'", "'other'", true),
    ] {
        let source = format!(
            "Base <- S7::new_class('Base', package = {parent}, properties = list(id = S7::class_integer))\n\
             Child <- S7::new_class('Child', parent = Base, package = {child}, properties = list(label = S7::class_character))\n"
        );
        let first = if forwards {
            ("...", "")
        } else {
            ("id", "integer(0)")
        };
        assert_constructor_parameters(
            &source,
            Some("0.2.2.9000"),
            "Child",
            &[first, ("label", "character(0)")],
        );
    }
}

#[test]
fn s7_constructor_keeps_builtin_s3_property_defaults() {
    let source = r#"
Event <- S7::new_class("Event", properties = list(
    date = S7::class_Date, zone = S7::class_POSIXct, id = S7::class_integer
))
"#;
    for (floor, date, zone) in [
        (
            None,
            "S7::class_Date$constructor()",
            "S7::class_POSIXct$constructor()",
        ),
        (
            Some("0.2.2.9000"),
            ".Date(numeric())",
            ".POSIXct(numeric(), tz = \"\")",
        ),
    ] {
        assert_constructor_parameters(
            source,
            floor,
            "Event",
            &[("date", date), ("zone", zone), ("id", "integer(0)")],
        );
    }
}

fn assert_constructor_parameters(
    source: &str,
    floor: Option<&str>,
    name: &str,
    expected: &[(&str, &str)],
) {
    at_calls(source, floor, |index, _, _, _| {
        let (id, _) = index
            .declarations
            .iter()
            .find(|(_, class)| class.declaration.display_name() == name)
            .unwrap();
        let signature = index.s7_api(id).unwrap().constructor.expect("constructor");
        let parameters: Vec<_> = signature
            .parameters
            .iter()
            .map(|parameter| {
                (
                    parameter.name.as_str(),
                    parameter.default.as_deref().unwrap_or_default(),
                )
            })
            .collect();
        assert_eq!(parameters, expected);
    });
}

#[test]
fn s7_dispatch_does_not_treat_property_types_as_exact() {
    let source = r#"
Parent <- S7::new_class("Parent")
Child <- S7::new_class("Child", parent = Parent)
Wrapper <- S7::new_class("Wrapper", properties = list(item = Parent))
g <- S7::new_generic("g", "x")
parent_method <- function(x) "parent"
child_method <- function(x) "child"
S7::method(g, Parent) <- parent_method
S7::method(g, Child) <- child_method
w <- Wrapper(item = Child())
item <- w@item
g(Child())
g(w@item)
g(S7::prop(w, "item"))
g(item)
"#;
    for floor in [None, Some("0.2.2.9000")] {
        assert_dispatch(source, floor, &[Some("child_method"), None, None, None]);
    }
}

#[test]
fn s7_super_uses_the_target_dispatch_vector() {
    let source = r#"
Root <- S7::new_S3_class("root")
Branch <- S7::new_S3_class("branch")
BranchWithRoot <- S7::new_S3_class(c("branch", "root"), constructor = function(.data = list()) structure(.data, class = c("branch", "root")))
Leaf <- S7::new_class("Leaf", parent = BranchWithRoot)
g <- S7::new_generic("g", "x")
root_method <- function(x) "root"
fallback <- function(x) "fallback"
S7::method(g, Root) <- root_method
S7::method(g, S7::class_any) <- fallback
g(S7::super(Leaf(), Branch))
g(S7::super(Leaf(), BranchWithRoot))
"#;
    for floor in [None, Some("0.2.2.9000")] {
        assert_dispatch(source, floor, &[Some("fallback"), Some("root_method")]);
    }
}

#[test]
fn s7_dispatch_recognizes_the_s7_object_tag_after_s4_ancestors() {
    let source = r#"
Base <- methods::setClass("Base", slots = c(b = "integer"))
Derived <- S7::new_class("Derived", parent = methods::getClass("Base"))
g <- S7::new_generic("g", "x")
object <- function(x) "object"
fallback <- function(x) "fallback"
S7::method(g, S7::S7_object) <- object
S7::method(g, S7::class_any) <- fallback
g(Derived())
"#;
    assert_dispatch(source, Some("0.2.2.9000"), &[Some("object")]);
}

#[test]
fn s7_dispatch_expands_the_language_union() {
    let source = r#"
Call <- S7::new_class("Call", parent = S7::class_call)
Name <- S7::new_class("Name", parent = S7::class_name)
g <- S7::new_generic("g", "x")
language <- function(x) "language"
fallback <- function(x) "fallback"
S7::method(g, S7::class_language) <- language
S7::method(g, S7::class_any) <- fallback
g(Call())
g(Name())
"#;
    for floor in [None, Some("0.2.2.9000")] {
        assert_dispatch(source, floor, &[Some("language"), Some("language")]);
    }
}

fn assert_dispatch(source: &str, floor: Option<&str>, expected: &[Option<&str>]) {
    use arity::ast::{AstNode, CallExpr};
    use arity::project::classes::MethodImplementationSource;

    at_calls(source, floor, |index, root, model, path| {
        let mut calls = root
            .children()
            .filter_map(CallExpr::cast)
            .filter(|call| call.callee_name().as_deref() == Some("g"));
        for expected in expected {
            let call = calls.next().unwrap();
            let dispatch = index.s7_dispatch_at(path, root, model, &call).unwrap();
            assert_eq!(
                dispatch.complete,
                expected.is_some(),
                "{call:?}: {dispatch:?}"
            );
            let implementation = dispatch
                .selected
                .and_then(|id| index.s7_registration(&id)?.implementation);
            assert_eq!(
                implementation,
                expected.map(|name| MethodImplementationSource::Binding {
                    owner: Default::default(),
                    name: name.to_string(),
                }),
                "{call:?}"
            );
        }
        assert!(calls.next().is_none());
    });
}

#[test]
fn s7_dispatch_checks_signature_list_provenance_in_the_package() {
    use arity::ast::{AstNode, CallExpr};
    use arity::incremental::parsed_tree_root;
    use arity::project::PackageInfo;

    let mut db = IncrementalDatabase::default();
    let pkg = Path::new("/s7-shadowed-list");
    let path = pkg.join("R/g.R");
    let sibling = db.upsert_file(
        &pkg.join("R/helper.R"),
        "list <- function(x, y) base::list(y, x)\n",
    );
    let source = r#"
g <- S7::new_generic("g", c("x", "y"))
specific <- function(x, y) "specific"
fallback <- function(x, y) "fallback"
S7::method(g, list(S7::class_integer, S7::class_character)) <- specific
S7::method(g, base::list(S7::class_any, S7::class_any)) <- fallback
g("text", 1L)
"#;
    let file = db.upsert_file(&path, source);
    db.set_workspace_members(vec![sibling, file], vec![pkg.into()]);
    db.set_package_metadata(vec![PackageInfo {
        root: pkg.into(),
        namespace: Some(String::new()),
        dir_sources: Default::default(),
        native_routines: Default::default(),
    }]);
    db.upsert_description(
        pkg,
        "Package: shapes\nImports: S7\nCollate: 'helper.R' 'g.R'\n",
    );
    let inspect = |db: &IncrementalDatabase| {
        let root = parsed_tree_root(db, file);
        let model = db.semantic_model(file);
        let index = db.snapshot().class_index_in(
            &path,
            &arity::project::classes::file_class_defs(model, &root),
            &root,
            model,
        );
        let call = root.children().filter_map(CallExpr::cast).last().unwrap();
        index.s7_dispatch_at(&path, &root, model, &call).unwrap()
    };
    let shadowed = inspect(&db);
    assert!(!shadowed.complete);
    assert!(shadowed.selected.is_none());
    db.set_file_text(file, source.replace("g, list(", "g, base::list("));
    let qualified = inspect(&db);
    assert!(qualified.complete);
    assert!(qualified.selected.is_some());
}

#[test]
fn incomplete_s7_properties_keep_observed_field_completions() {
    use lsp_types::{CompletionItemKind, CompletionResponse};

    let source = r#"
build_props <- function() list(known = S7::class_integer)
Parent <- S7::new_class("Parent", properties = list(id = S7::class_integer))
Example <- S7::new_class("Example", parent = Parent, properties = build_props())
obj <- Example()
obj@known
obj@id
obj@
"#;
    let offset = source.rfind('@').unwrap() + 1;
    let CompletionResponse::List(items) =
        arity::lsp::compute_completions(source, offset, &Default::default()).unwrap()
    else {
        panic!("completion list");
    };
    assert!(items.is_incomplete);
    assert!(items.items.iter().any(|item| item.label == "known"));
    let ids: Vec<_> = items
        .items
        .iter()
        .filter(|item| item.label == "id")
        .collect();
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0].kind, Some(CompletionItemKind::PROPERTY));
}

#[test]
fn s7_union_types_supply_receiver_members() {
    let definitions = r#"
A <- S7::new_class("A", properties = list(first = S7::class_integer))
B <- S7::new_class("B", properties = list(second = S7::class_integer))
U <- S7::new_union(A, B)
Wrapper <- S7::new_class("Wrapper", properties = list(item = U))
"#;
    for expression in [
        "Wrapper()@item@|",
        "S7::prop(Wrapper(), 'item')@|",
        "g <- S7::new_generic('g', 'x')\nS7::method(g, U) <- function(x) x@|",
    ] {
        let source = format!("{definitions}{expression}");
        let offset = source.find('|').unwrap();
        let lsp_types::CompletionResponse::List(items) =
            arity::lsp::compute_completions(&source.replace('|', ""), offset, &Default::default())
                .unwrap()
        else {
            panic!("completion list")
        };
        for name in ["first", "second"] {
            assert!(
                items.items.iter().any(|item| item.label == name),
                "{expression}: {name}"
            );
        }
    }
}

#[test]
fn s7_union_preserves_known_classes_with_unmodeled_alternatives() {
    use arity::project::classes::file_class_defs;
    use arity::semantic::SemanticModel;

    let source = r#"
A <- S7::new_class("A", properties = list(first = S7::class_integer))
U <- S7::new_union(A, S7::class_integer)
Wrapper <- S7::new_class("Wrapper", properties = list(item = U))
item <- Wrapper()@item
item"#;
    let root = arity::parser::parse(source).cst;
    let model = SemanticModel::build(&root);
    let path = Path::new("/s7-union.R");
    let index = IncrementalDatabase::default().snapshot().class_index_in(
        path,
        &file_class_defs(&model, &root),
        &root,
        &model,
    );
    let value = root.last_token().unwrap().into();
    let evidence = index.class_receiver_evidence(path, &root, &model, &value);
    assert!(evidence.unknown);
    let [candidate] = evidence.candidates.as_slice() else {
        panic!("{evidence:?}");
    };
    assert_eq!(
        index.declarations[&candidate.class].declaration.name,
        ClassName::Known("A".into())
    );
    assert!(index.class_receiver(path, &root, &model, &value).is_none());
}

#[test]
fn s7_property_validators_complete_members_of_the_value_type() {
    use lsp_types::CompletionResponse;

    for (class, expected) in [("Value", Some("inner")), ("build_type()", None)] {
        let source = format!(
            r#"
Value <- S7::new_class("Value", properties = list(inner = S7::class_integer))
Wrapper <- S7::new_class("Wrapper", properties = list(
    outside = S7::class_integer,
    item = S7::new_property({class}, validator = function(value) {{
        invisible(value@)
        NULL
    }})
))
"#
        );
        let offset = source.rfind('@').unwrap() + 1;
        let CompletionResponse::List(items) =
            arity::lsp::compute_completions(&source, offset, &Default::default()).unwrap()
        else {
            panic!("completion list");
        };
        let labels: Vec<_> = items.items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(labels, expected.into_iter().collect::<Vec<_>>(), "{class}");
    }
}

#[test]
fn resolved_s7_facts_backdate_across_bodies_and_refresh_on_version_changes() {
    use arity::incremental::QueryKind;
    use arity::project::project_classes;
    let mut db = IncrementalDatabase::default();
    let path = Path::new("/s7-incremental/R/classes.R");
    let package = path.parent().unwrap().parent().unwrap();
    let source = "Point <- S7::new_class('Point', properties = list(x = S7::new_property(S7::class_integer, getter = function(self) 1L)))\n";
    let file = db.upsert_file(path, source);
    db.set_workspace_members(vec![file], vec![package.to_path_buf()]);
    db.refresh_descriptions([package.to_path_buf()]);
    db.upsert_description(package, "Package: shapes\nImports: S7 (>= 0.2.2)\n");
    assert_eq!(project_classes(&db).declarations.len(), 1);
    db.set_file_text(file, source.replace("1L", "2L"));
    db.clear_query_log();
    assert_eq!(project_classes(&db).declarations.len(), 1);
    assert!(
        db.query_log()
            .iter()
            .all(|entry| entry.kind != QueryKind::ProjectClasses)
    );
    db.set_file_text(file, source.replace("class_integer", "class_double"));
    db.clear_query_log();
    assert_eq!(project_classes(&db).declarations.len(), 1);
    assert!(
        db.query_log()
            .iter()
            .any(|entry| entry.kind == QueryKind::ProjectClasses)
    );
    db.upsert_description(package, "Package: shapes\nImports: S7 (>= 0.2.2.9000)\n");
    assert_eq!(
        project_classes(&db).s7_version(path),
        arity::project::classes::S7Version::Development
    );
}

#[test]
fn external_registration_is_applied_in_its_own_loaded_namespace() {
    use arity::ast::{AstNode, CallExpr};
    use arity::project::PackageInfo;
    use arity::project::classes::{S7GenericTarget, S7HookKind};

    let mut db = IncrementalDatabase::default();
    let provider = Path::new("/s7-provider");
    let extension = Path::new("/s7-extension");
    let loader = Path::new("/s7-loader");
    let main = Path::new("/s7-main/run.R");
    let source = "provider::probe(extension::Child())\n";
    let target = db.upsert_file(
        &provider.join("R/defs.R"),
        "probe <- S7::new_generic('probe', 'x')\nS7::method(probe, S7::class_any) <- function(x) 0L\n",
    );
    let extension_source = r#"
Child <- S7::new_class("Child")
ext <- S7::new_external_generic("provider", "probe", "x", "1.0.0")
S7::method(ext, Child) <- function(x) "extension"
S7::method(ext, S7::class_integer) <- function(x) "integer extension"
.onLoad <- function(...) S7::methods_register()
"#;
    let child = db.upsert_file(&extension.join("R/defs.R"), extension_source);
    let script = db.upsert_file(main, source);
    let loader_file = db.upsert_file(&loader.join("R/hooks.R"), "loaded <- TRUE\n");
    db.set_workspace_members(
        vec![target, child, script, loader_file],
        vec![provider.into(), extension.into(), loader.into()],
    );
    db.set_package_metadata(
        [
            (provider, "export(probe)"),
            (extension, "export(Child)"),
            (loader, "import(extension)"),
        ]
        .into_iter()
        .map(|(root, ns)| PackageInfo {
            root: root.into(),
            namespace: Some(ns.into()),
            dir_sources: Default::default(),
            native_routines: Default::default(),
        })
        .collect(),
    );
    db.refresh_descriptions([provider.into(), extension.into(), loader.into()]);
    db.upsert_description(loader, "Package: loader\nImports: extension\n");
    db.upsert_description(provider, "Package: provider\nVersion: 1.0.0\n");
    db.upsert_description(
        extension,
        "Package: extension\nImports: S7\nSuggests: provider\n",
    );
    for (text, active) in [
        (format!("library(extension)\n{source}"), true),
        (source.to_string(), true),
        ("library(loader)\nprovider::probe(1L)\n".into(), true),
        ("provider::probe(1L)\n".to_string(), false),
    ] {
        db.set_file_text(script, text);
        let root = arity::incremental::parsed_tree_root(&db, script);
        let model = db.semantic_model(script);
        let facts = arity::project::classes::file_class_defs(model, &root);
        let index = db.snapshot().class_index_in(main, &facts, &root, model);
        assert!(index.s7_has_hook(extension, S7HookKind::Load));
        let call = root.children().filter_map(CallExpr::cast).last().unwrap();
        let dispatch = index.s7_dispatch_at(main, &root, model, &call).unwrap();
        if active {
            let method = index.s7_registration(&dispatch.selected.unwrap()).unwrap();
            assert!(method.delayed);
            assert_eq!(method.owner_package.as_deref(), Some("extension"));
            assert!(
                matches!(method.targets.as_slice(), [S7GenericTarget::S7(target)] if target.path.starts_with(provider))
            );
        } else {
            let method = index.s7_registration(&dispatch.selected.unwrap()).unwrap();
            assert!(!method.delayed);
        }
    }
}

#[test]
fn s7_dispatch_checks_the_value_of_l_suffixed_literals() {
    let source = r#"
g <- S7::new_generic("g", "x")
int <- function(x) 1
dbl <- function(x) 2
S7::method(g, S7::class_integer) <- int
S7::method(g, S7::class_double) <- dbl
g(1.5L)
g(2147483648L)
g(1e2L)
g(1.0L)
g(0x1L)
"#;
    assert_dispatch(
        source,
        None,
        &[
            Some("dbl"),
            Some("dbl"),
            Some("int"),
            Some("int"),
            Some("int"),
        ],
    );
}

#[test]
fn s7_dispatch_withholds_exact_types_for_custom_constructors() {
    let source = r#"
C <- S7::new_class("C", constructor = function(flag) {
    if (flag) S7::new_object(S7::S7_object()) else 1L
})
g <- S7::new_generic("g", "x")
as_c <- function(x) 1
as_int <- function(x) 2
S7::method(g, C) <- as_c
S7::method(g, S7::class_integer) <- as_int
g(C(FALSE))
"#;
    assert_dispatch(source, None, &[None]);
    assert_constructor_parameters(source, None, "C", &[("flag", "")]);
}

#[test]
fn s7_constructor_inherits_builtin_s3_parent_formals() {
    let source = r#"
Event <- S7::new_class("Event", parent = S7::class_Date,
                       properties = list(label = S7::class_character))
"#;
    for floor in [None, Some("0.2.2.9000")] {
        assert_constructor_parameters(
            source,
            floor,
            "Event",
            &[(".data", "double()"), ("label", "character(0)")],
        );
    }
}

#[test]
fn s7_development_registers_descendants_of_s4_classes() {
    let source = r#"
methods::setClass("Formal")
Middle <- S7::new_class("Middle", parent = methods::getClass("Formal"))
Leaf <- S7::new_class("Leaf", parent = Middle)
"#;
    at_calls(source, Some("0.2.2.9000"), |index, _, _, _| {
        let (leaf, _) = index
            .declarations
            .iter()
            .find(|(_, class)| class.declaration.display_name() == "Leaf")
            .unwrap();
        assert!(index.s7_registered_with_s4(leaf));
    });
}

#[test]
fn s7_injected_property_name_cannot_be_renamed_as_a_local_variable() {
    let mut db = IncrementalDatabase::default();
    let file = db.upsert_file(Path::new("/s7-rename.R"),
        "library(S7)\nradius := new_property(class_double)\nCircle := new_class(properties = list(radius))");
    db.set_s7_floor(file, arity::config::CompatVersion::parse("0.2.2.9000"));
    let model = db.semantic_model(file);
    let read = model
        .idents()
        .iter()
        .find(|ident| ident.name == "radius")
        .unwrap();
    let resolution = model.identifier_resolution(read);
    assert_eq!(resolution.lexical.len(), 1);
    assert!(!model.binding_rename_is_safe(resolution.lexical[0]));
}

#[test]
fn autofix_and_lint_use_the_same_s7_config_for_sibling_bindings() {
    use std::{fs, process::Command};

    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path();
    fs::create_dir(pkg.join("R")).unwrap();
    fs::write(pkg.join("DESCRIPTION"), "Package: shapes\nImports: S7\n").unwrap();
    fs::write(pkg.join("NAMESPACE"), "import(S7)\n").unwrap();
    fs::write(pkg.join("R/shadow.R"), "length := S7::new_generic('x')\n").unwrap();

    let floor = "[compat]\ns7 = '0.2.2.9000'\n";
    fs::write(pkg.join("explicit.toml"), floor).unwrap();
    let path = pkg.join("R/use.R");
    let original = "x <- c(1, 2)\nprint(1:length(x))\n";
    for (args, expected) in [
        (&["--config", "explicit.toml"][..], original),
        (&["--no-config"][..], "x <- c(1, 2)\nprint(seq_along(x))\n"),
    ] {
        let disk = if args[0] == "--no-config" { floor } else { "" };
        fs::write(pkg.join("arity.toml"), disk).unwrap();
        fs::write(&path, original).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_arity"))
            .current_dir(pkg)
            .args(["lint", "--fix", "--select", "seq"])
            .args(args)
            .arg("R/use.R")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    }
}

#[test]
fn excluded_package_sources_do_not_load_invalid_config() {
    use std::{fs, process::Command};

    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path();
    fs::create_dir(pkg.join("R")).unwrap();
    fs::create_dir(pkg.join("tests")).unwrap();
    fs::write(pkg.join("DESCRIPTION"), "Package: shapes\nImports: S7\n").unwrap();
    fs::write(pkg.join("NAMESPACE"), "import(S7)\n").unwrap();
    fs::write(
        pkg.join("arity.toml"),
        "exclude = ['R/']\n[compat]\ns7 = '0.2.2.9000'\n",
    )
    .unwrap();
    fs::write(pkg.join("R/arity.toml"), "invalid TOML").unwrap();
    fs::write(pkg.join("R/classes.R"), "Point := S7::new_class()\n").unwrap();
    fs::write(pkg.join("tests/use.R"), "Point()\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_arity"))
        .current_dir(pkg)
        .args(["lint", "--select", "undefined-symbol", "tests/use.R"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn s7_external_parent_is_in_the_shared_inheritance_graph() {
    use arity::project::PackageInfo;
    let mut db = IncrementalDatabase::default();
    let package = Path::new("/s7-parents");
    let parent = db.upsert_file(
        &package.join("R/parent.R"),
        "Parent <- S7::new_class('Parent')\n",
    );
    let path = Path::new("/s7-child/child.R");
    let child = db.upsert_file(
        path,
        "Child <- S7::new_class('Child', parent = S7::new_external_class('parents', 'Parent'))\n",
    );
    db.set_s7_floor(child, arity::config::CompatVersion::parse("0.2.2.9000"));
    db.set_workspace_members(
        vec![parent, child],
        vec![package.into(), "/s7-child".into()],
    );
    db.set_package_metadata(vec![PackageInfo {
        root: package.into(),
        namespace: Some("export(Parent)\n".into()),
        dir_sources: Default::default(),
        native_routines: Default::default(),
    }]);
    db.refresh_descriptions([package.into()]);
    db.upsert_description(package, "Package: parents\n");
    let root = arity::incremental::parsed_tree_root(&db, child);
    let model = db.semantic_model(child);
    let facts = arity::project::classes::file_class_defs(model, &root);
    let index = db.snapshot().class_index_in(path, &facts, &root, model);
    let class = |name| {
        index
            .declarations
            .iter()
            .find(|(_, declaration)| declaration.declaration.display_name() == name)
            .map(|(id, _)| id)
            .unwrap()
    };
    assert!(index.supertypes[class("Child")].contains(class("Parent")));
    assert!(index.subtypes[class("Parent")].contains(class("Child")));
}

#[test]
fn s7_union_alias_expansion_is_bounded() {
    use std::fmt::Write as _;
    let mut source = "U0 <- S7::new_union(S7::class_integer, S7::class_double)\n".to_string();
    for i in 1..=10 {
        writeln!(source, "U{i} <- S7::new_union(U{}, U{})", i - 1, i - 1).unwrap();
    }
    source.push_str("C <- S7::new_class('C', properties = list(x = U10))\n");
    at_calls(&source, None, |index, _, _, _| {
        let (id, _) = index
            .declarations
            .iter()
            .find(|(_, declaration)| declaration.declaration.display_name() == "C")
            .unwrap();
        let api = index.s7_api(id).unwrap();
        assert_eq!(
            api.properties[0].definition.class,
            Some(arity::project::classes::S7ClassSpec::Unknown)
        );
    });
}
