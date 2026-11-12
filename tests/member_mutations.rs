use std::path::Path;

use arity::incremental::IncrementalDatabase;
use arity::project::classes::{ClassId, ClassIndex, MemberKind};

fn index(source: &str) -> ClassIndex {
    let mut db = IncrementalDatabase::default();
    let file = db.upsert_file(Path::new("/member-mutations/R/a.R"), source);
    db.set_workspace_members(vec![file], vec!["/member-mutations".into()]);
    db.set_package_metadata(vec![arity::project::PackageInfo {
        root: "/member-mutations".into(),
        namespace: None,
        dir_sources: Default::default(),
        native_routines: Default::default(),
    }]);
    db.snapshot().class_index()
}

fn class(index: &ClassIndex, binding: &str) -> ClassId {
    index
        .declarations
        .iter()
        .find(|(_, class)| class.declaration.binding.as_deref() == Some(binding))
        .unwrap()
        .0
        .clone()
}

#[test]
fn r6_set_tracks_aliases_replacements_private_and_active_members() {
    let index = index(
        r#"
G <- R6::R6Class(public = list(read = function(old = 1) old))
Alias <- G
Alias$set("public", "read", function(scale = 2) self$value * scale, overwrite = TRUE)
G$set(name = "value", value = 3, which = "public")
G$set("private", "secret", function(x = 4) x)
G$set("active", "doubled", function(value) self$value * 2)
"#,
    );
    let api = index.r6_api(&class(&index, "G")).unwrap();
    assert!(api.public.complete && api.private.complete);
    assert_eq!(
        api.public.entries["read"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .name,
        "scale"
    );
    assert_eq!(api.public.entries["value"][0].kind, MemberKind::Field);
    assert_eq!(api.private.entries["secret"][0].kind, MemberKind::Method);
    assert_eq!(
        api.public.entries["doubled"][0].kind,
        MemberKind::ActiveBinding
    );
}

#[test]
fn r6_set_respects_class_locks_and_overwrite() {
    let index = index(
        r#"
G <- R6::R6Class(lock_class = TRUE, public = list(read = function(original) original))
G$set("public", "blocked", 1)
G$unlock()
G$set("public", "read", function(rejected) rejected)
G$set("public", "allowed", 2)
G$lock()
G$set("public", "blocked_again", 3)
"#,
    );
    let api = index.r6_api(&class(&index, "G")).unwrap();
    assert!(!api.public.entries.contains_key("blocked"));
    assert!(!api.public.entries.contains_key("blocked_again"));
    assert!(api.public.entries.contains_key("allowed"));
    assert_eq!(
        api.public.entries["read"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .name,
        "original"
    );
}

#[test]
fn reference_methods_add_replace_remove_and_preserve_field_locks() {
    let index = index(
        r#"
G <- methods::setRefClass("MutationReference", fields = "value", methods = list(read = function(old) old, removed = function() 1))
Alias <- G
Alias$methods(read = function(scale = 2) value * scale, added = function(x = 3) x)
G$methods(removed = NULL)
G$lock("value")
G$methods(after_lock = function() value)
"#,
    );
    let api = index.reference_api(&class(&index, "G")).unwrap();
    assert!(api.instance.complete);
    assert_eq!(
        api.instance.entries["read"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .name,
        "scale"
    );
    assert!(api.instance.entries.contains_key("added"));
    assert!(api.instance.entries.contains_key("after_lock"));
    // R reintroduces an own method while rebuilding the method table.
    assert!(api.instance.entries.contains_key("removed"));
}

#[test]
fn dynamic_mutations_and_custom_parent_environments_are_incomplete() {
    for source in [
        "G <- R6::R6Class(); G$set(which, name, value)",
        "G <- R6::R6Class(); if (flag) G$set('public', 'x', 1)",
        "G <- R6::R6Class(); mutate <- function() G$set('public', 'x', 1)",
        "G <- R6::R6Class(parent_env = environment())",
    ] {
        let index = index(source);
        assert!(
            !index.r6_api(&class(&index, "G")).unwrap().public.complete,
            "{source}"
        );
    }
    let index = index("G <- methods::setRefClass('DynamicReference'); G$methods(members)");
    assert!(
        !index
            .reference_api(&class(&index, "G"))
            .unwrap()
            .instance
            .complete
    );
}

fn help(marked: &str) -> Option<String> {
    let offset = marked.find('|').unwrap();
    arity::lsp::compute_signature_help(
        &marked.replacen('|', "", 1),
        offset,
        &arity::rindex::provider::IndexedProvider::empty(),
    )
    .map(|help| help.signatures[0].label.clone())
}

#[test]
fn editor_signatures_follow_mutation_order_and_instance_creation() {
    let before = "G <- R6::R6Class(public = list(read = function(before) before))\n";
    let after = "G$set('public', 'read', function(after) after, overwrite = TRUE)\n";
    assert_eq!(
        help(&format!("{before}G$new()$read(|)\n{after}")),
        Some("read(before)".into())
    );
    assert_eq!(
        help(&format!("{before}{after}G$new()$read(|)")),
        Some("read(after)".into())
    );
    assert_eq!(
        help(&format!("{before}old <- G$new()\n{after}old$read(|)")),
        Some("read(before)".into())
    );
    assert_eq!(
        help(&format!("{before}old <- G$new()\n{after}G$new()$read(|)")),
        Some("read(after)".into())
    );
}

#[test]
fn members_installed_from_another_file_follow_collate_order() {
    let mut db = IncrementalDatabase::default();
    let root = Path::new("/member-mutations");
    let source = db.upsert_file(&root.join("R/z-definition.R"), "G <- R6::R6Class()\n");
    let implementation = db.upsert_file(
        &root.join("R/y-function.R"),
        "implementation <- function(scale = 5) self$value * scale\n",
    );
    let addition = db.upsert_file(
        &root.join("R/a-addition.R"),
        "G$set('public', 'read', implementation)\nG$set('public', 'value', 3)\n",
    );
    db.set_workspace_members(vec![source, implementation, addition], vec![root.into()]);
    db.set_package_metadata(vec![arity::project::PackageInfo {
        root: root.into(),
        namespace: None,
        dir_sources: ["z-definition.R", "y-function.R", "a-addition.R"]
            .map(str::to_owned)
            .into(),
        native_routines: Default::default(),
    }]);
    db.refresh_descriptions([root.into()]);
    db.upsert_description(
        root,
        "Package: mutationtest\nCollate: 'z-definition.R' 'y-function.R' 'a-addition.R'\n",
    );
    let index = db.snapshot().class_index();
    let api = index.r6_api(&class(&index, "G")).unwrap();
    assert!(api.public.complete);
    let member = &api.public.entries["read"][0];
    assert_eq!(
        member.declaration.as_ref().unwrap().path,
        root.join("R/a-addition.R")
    );
    assert_eq!(
        member.signature.as_ref().unwrap().parameters[0]
            .default
            .as_deref(),
        Some("5")
    );

    db.upsert_description(
        root,
        "Package: mutationtest\nCollate: 'a-addition.R' 'z-definition.R' 'y-function.R'\n",
    );
    let index = db.snapshot().class_index();
    let api = index.r6_api(&class(&index, "G")).unwrap();
    assert!(!api.public.entries.contains_key("read"));
}

#[test]
fn r6_set_checks_own_member_groups_and_preserves_null_fields() {
    let index = index(
        r#"
Base <- R6::R6Class(public = list(inherited = function(base) base))
G <- R6::R6Class(inherit = Base, public = list(read = function(original) original))
G$set("public", "inherited", function(child) child)
G$set("public", "read", 3, overwrite = TRUE)
G$set("public", "null_field", NULL)
"#,
    );
    let api = index.r6_api(&class(&index, "G")).unwrap();
    assert_eq!(
        api.public.entries["inherited"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .name,
        "child"
    );
    // R6 permits overwriting only within the same field/method/active group.
    assert_eq!(api.public.entries["read"][0].kind, MemberKind::Method);
    assert_eq!(api.public.entries["null_field"][0].kind, MemberKind::Field);
}

#[test]
fn reference_method_lists_and_recovered_generators_are_supported() {
    let index = index(
        r#"
G <- methods::setRefClass("RecoveredMutation")
methods::getRefClass("RecoveredMutation")$methods(list(read = function(scale = 2) scale))
"#,
    );
    let api = index.reference_api(&class(&index, "G")).unwrap();
    assert!(api.instance.complete);
    assert_eq!(
        api.instance.entries["read"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .name,
        "scale"
    );
}

#[test]
fn mutation_method_enclosures_supply_self_private_and_reference_fields() {
    let source = r#"
G <- R6::R6Class()
G$set("private", "secret", 1)
G$set("public", "read", function() self$value + private$secret)
Ref <- methods::setRefClass("EnclosureMutation", fields = "value")
Ref$methods(read = function() { value <<- value + 1; value })
print(G)
print(Ref)
"#;
    let config = arity::config::LintConfig {
        select: Some(vec!["undefined-symbol".into()]),
        ..Default::default()
    };
    let diagnostics = arity::linter::check_document(Path::new("test.R"), source, &config).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(
        help(&format!("{source}\nRef()$read(|)")),
        Some("read()".into())
    );
}

#[test]
fn reference_class_binding_locks_block_method_installation() {
    let index = index(
        r#"
G <- methods::setRefClass("LockedMutation")
lockBinding(".__C__LockedMutation", environment())
G$methods(blocked = function() 1)
"#,
    );
    let api = index.reference_api(&class(&index, "G")).unwrap();
    assert!(!api.instance.entries.contains_key("blocked"));
}

#[test]
fn source_calls_replay_mutations_at_their_execution_position() {
    let mut db = IncrementalDatabase::default();
    let path = Path::new("/mutation-scripts/main.R");
    let main = db.upsert_file(path, "G <- R6::R6Class()\nsource('addition.R')\nG$set('public', 'read', function(last) last, overwrite = TRUE)\n");
    let addition = db.upsert_file(
        Path::new("/mutation-scripts/addition.R"),
        "G$set('public', 'read', function(first) first)\n",
    );
    db.set_workspace_members(vec![main, addition], vec!["/mutation-scripts".into()]);
    let index = db.snapshot().class_index();
    let api = index.r6_api(&class(&index, "G")).unwrap();
    assert!(api.public.complete);
    assert_eq!(
        api.public.entries["read"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .name,
        "last"
    );
}

#[test]
fn unrelated_or_shadowed_receivers_cannot_mutate_a_class() {
    let index = index(
        r#"
G <- R6::R6Class(public = list(original = 1))
other <- list(set = function(...) NULL)
other$set("public", "unrelated", 2)
f <- function(G) G$set("public", "shadowed", 3)
object <- G$new()
object$set("public", "instance", 4)
"#,
    );
    let api = index.r6_api(&class(&index, "G")).unwrap();
    assert!(api.public.complete);
    for name in ["unrelated", "shadowed", "instance"] {
        assert!(!api.public.entries.contains_key(name));
    }
}

#[test]
fn accessor_reads_and_writes_are_executable_and_object_locks_remain_distinct() {
    use arity::project::classes::{AccessorArgument, ClassAccess, ClassReceiver, MemberEffect};
    let index = index(
        r#"
G <- R6::R6Class(public = list(value = 1, read = function() self$value))
G$set("active", "computed", function(value) stop("must never execute"))
Ref <- methods::setRefClass("AccessMutation", fields = list(value = "numeric", computed = function(value) stop("must never execute")))
Ref$lock("value")
RefChild <- methods::setRefClass("AccessMutationChild", contains = "AccessMutation")
Open <- R6::R6Class(lock_objects = FALSE, public = list(read = function() 1))
"#,
    );
    for binding in ["G", "Ref", "RefChild"] {
        let receiver = ClassReceiver {
            class: class(&index, binding),
            access: ClassAccess::Public,
            point: None,
            cached_members: Default::default(),
            uncertain_cache: false,
        };
        let access = index.member_access(&receiver, "computed").unwrap();
        assert!(matches!(
            access.read,
            MemberEffect::InvokeAccessor {
                implementation: Some(_),
                argument: AccessorArgument::Missing
            }
        ));
        assert!(matches!(
            access.write,
            MemberEffect::InvokeAccessor {
                implementation: Some(_),
                argument: AccessorArgument::AssignedValue
            }
        ));
        let access = index.member_access(&receiver, "value").unwrap();
        assert_eq!(access.read, MemberEffect::Stored);
        assert_eq!(
            access.write,
            if binding == "G" {
                MemberEffect::Stored
            } else {
                MemberEffect::InitializeOnce
            }
        );
        if binding == "G" {
            assert_eq!(
                index.member_access(&receiver, "read").unwrap().write,
                MemberEffect::ReadOnly
            );
        }
    }
    let receiver = ClassReceiver {
        class: class(&index, "Open"),
        access: ClassAccess::Public,
        point: None,
        cached_members: Default::default(),
        uncertain_cache: false,
    };
    assert_eq!(
        index.member_access(&receiver, "read").unwrap().write,
        MemberEffect::Stored
    );
}

#[test]
fn inheritance_uses_reference_declaration_time_and_r6_construction_time() {
    let index = index(
        r#"
Parent <- methods::setRefClass("MutationParent", methods = list(read = function(before) before))
Child <- methods::setRefClass("MutationChild", contains = "MutationParent")
Parent$methods(read = function(after) after, added = function() 1)
R6Parent <- R6::R6Class(public = list(read = function(before) before))
R6Child <- R6::R6Class(inherit = R6Parent)
R6Parent$set("public", "read", function(after) after, overwrite = TRUE)
"#,
    );
    let reference = index.reference_api(&class(&index, "Child")).unwrap();
    assert!(!reference.instance.entries.contains_key("added"));
    assert_eq!(
        reference.instance.entries["read"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .name,
        "before"
    );
    let r6 = index.r6_api(&class(&index, "R6Child")).unwrap();
    assert_eq!(
        r6.public.entries["read"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .name,
        "after"
    );
}

#[test]
fn external_implementation_signatures_and_enclosures_refresh_across_files() {
    let mut db = IncrementalDatabase::default();
    let root = Path::new("/external-mutations");
    let definitions = db.upsert_file(
        &root.join("R/a.R"),
        "G <- R6::R6Class()\nRef <- methods::setRefClass('ExternalMutation', fields = 'value')\n",
    );
    let implementation = db.upsert_file(&root.join("R/b.R"), "r6_impl <- function(scale = 2) self$value * scale\nref_impl <- function(scale = 3) value * scale\n");
    let mutation = db.upsert_file(
        &root.join("R/c.R"),
        "G$set('public', 'read', r6_impl)\nRef$methods(read = ref_impl)\n",
    );
    db.set_workspace_members(
        vec![definitions, implementation, mutation],
        vec![root.into()],
    );
    db.set_package_metadata(vec![arity::project::PackageInfo {
        root: root.into(),
        namespace: None,
        dir_sources: Default::default(),
        native_routines: Default::default(),
    }]);
    let before = db.snapshot().class_index();
    for (binding, expected) in [("G", "2"), ("Ref", "3")] {
        let signature = if binding == "G" {
            before
                .r6_api(&class(&before, binding))
                .unwrap()
                .public
                .entries["read"][0]
                .signature
                .clone()
        } else {
            before
                .reference_api(&class(&before, binding))
                .unwrap()
                .instance
                .entries["read"][0]
                .signature
                .clone()
        };
        assert_eq!(
            signature.unwrap().parameters[0].default.as_deref(),
            Some(expected)
        );
    }
    let environments = before.method_environments(&root.join("R/b.R"));
    assert!(
        environments
            .iter()
            .any(|environment| environment.implementations == ["r6_impl"])
    );
    assert!(
        environments
            .iter()
            .any(|environment| environment.implementations == ["ref_impl"])
    );
    db.set_file_text(implementation, "r6_impl <- function(scale = 8) self$value * scale\nref_impl <- function(scale = 9) value * scale\n");
    let after = db.snapshot().class_index();
    assert_eq!(
        after.r6_api(&class(&after, "G")).unwrap().public.entries["read"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .default
            .as_deref(),
        Some("8")
    );
}

#[test]
fn indirect_and_direct_generator_mutations_preserve_uncertainty() {
    for source in [
        "G <- R6::R6Class(); do.call(G$set, arguments)",
        "G <- R6::R6Class(); G$public_methods$read <- implementation",
        "G <- R6::R6Class(); G$parent_env <- other_environment",
    ] {
        let index = index(source);
        assert!(
            !index.r6_api(&class(&index, "G")).unwrap().public.complete,
            "{source}"
        );
    }
    let source = "G <- R6::R6Class(); setter <- G$set; setter('public', 'value', 1)";
    let index = index(source);
    assert!(
        index
            .r6_api(&class(&index, "G"))
            .unwrap()
            .public
            .entries
            .contains_key("value")
    );
}

#[test]
fn clone_replacements_and_cached_reference_methods_keep_their_actual_signatures() {
    assert_eq!(
        help(
            "G <- R6::R6Class()\nG$set('public', 'clone', function(other) other, overwrite = TRUE)\nG$new()$clone(|)"
        ),
        Some("clone(other)".into())
    );
    let before = "G <- methods::setRefClass('CachedMutation', methods = list(read = function(before) before))\nx <- G()\ncached <- x$read\n";
    let after = "G$methods(read = function(after) after)\n";
    assert_eq!(
        help(&format!("{before}{after}x$read(|)")),
        Some("read(before)".into())
    );
    assert_eq!(
        help(&format!("{before}{after}G()$read(|)")),
        Some("read(after)".into())
    );
}

#[test]
fn implementation_rebinding_uses_the_value_at_installation() {
    let index = index(
        r#"
G <- R6::R6Class()
implementation <- function(first) first
G$set("public", "before", implementation)
implementation <- function(second) second
G$set("public", "after", implementation)
"#,
    );
    let api = index.r6_api(&class(&index, "G")).unwrap();
    assert!(api.public.complete);
    assert_eq!(
        api.public.entries["before"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .name,
        "first"
    );
    assert_eq!(
        api.public.entries["after"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .name,
        "second"
    );
}

#[test]
fn conditional_installations_keep_the_method_enclosure_without_claiming_members() {
    let source = "G <- R6::R6Class()\nif (flag) G$set('public', name, function() self$value)\n";
    let config = arity::config::LintConfig {
        select: Some(vec!["undefined-symbol".into()]),
        ..Default::default()
    };
    let diagnostics = arity::linter::check_document(Path::new("test.R"), source, &config).unwrap();
    assert!(
        !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.body.contains("`self`")),
        "{diagnostics:?}"
    );
    let index = index(source);
    assert!(!index.r6_api(&class(&index, "G")).unwrap().public.complete);
}

#[test]
fn qualified_workspace_implementations_keep_their_namespace_identity() {
    let mut db = IncrementalDatabase::default();
    let implementation = db.upsert_file(
        Path::new("/implementation/R/a.R"),
        "read <- function(scale = 7) self$value * scale\n",
    );
    let consumer = db.upsert_file(
        Path::new("/consumer/R/a.R"),
        "G <- R6::R6Class()\nG$set('public', 'read', implementation::read)\n",
    );
    db.set_workspace_members(
        vec![implementation, consumer],
        vec!["/implementation".into(), "/consumer".into()],
    );
    db.refresh_descriptions(["/implementation".into(), "/consumer".into()]);
    db.upsert_description(Path::new("/implementation"), "Package: implementation\n");
    db.upsert_description(
        Path::new("/consumer"),
        "Package: consumer\nImports: implementation\n",
    );
    db.set_package_metadata(
        [("/implementation", "export(read)\n"), ("/consumer", "")]
            .map(|(root, namespace)| arity::project::PackageInfo {
                root: root.into(),
                namespace: Some(namespace.into()),
                dir_sources: Default::default(),
                native_routines: Default::default(),
            })
            .into(),
    );
    let index = db.snapshot().class_index();
    let api = index.r6_api(&class(&index, "G")).unwrap();
    assert!(api.public.complete);
    assert_eq!(
        api.public.entries["read"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .default
            .as_deref(),
        Some("7")
    );
}

#[test]
fn uncertain_class_lock_environments_do_not_prove_a_locked_definition() {
    for operation in [
        "lockit <- function() lockBinding('.__C__UncertainLock', globalenv())",
        "lockBinding('.__C__UncertainLock', chosen_environment)",
        "environment <- function() chosen_environment; lockBinding('.__C__UncertainLock', environment())",
    ] {
        let source = format!(
            "G <- methods::setRefClass('UncertainLock')\n{operation}\nG$methods(read = function(x) x)\n"
        );
        let index = index(&source);
        assert!(
            !index
                .reference_api(&class(&index, "G"))
                .unwrap()
                .instance
                .complete,
            "{source}"
        );
    }
}

#[test]
fn generator_alias_rebinding_follows_order_and_keeps_conditional_targets_incomplete() {
    let source = "G <- R6::R6Class()\nAlias <- G\nAlias <- G\nAlias$set('public', 'known', 1)\n";
    let index = index(source);
    assert!(
        index
            .r6_api(&class(&index, "G"))
            .unwrap()
            .public
            .entries
            .contains_key("known")
    );
    let source = "G <- R6::R6Class()\nAlias <- G\nif (flag) Alias <- other\nAlias$set('public', 'possible', function() self$value)\n";
    let index = self::index(source);
    let api = index.r6_api(&class(&index, "G")).unwrap();
    assert!(!api.public.complete);
    assert!(!api.public.entries.contains_key("possible"));
}

#[test]
fn installed_closures_keep_the_class_creation_frame_and_their_original_value() {
    for declaration in [
        "G <- R6::R6Class(public = list(read = helper))",
        "G <- R6::R6Class(); G$set('public', 'read', helper)",
    ] {
        let source = format!(
            "helper <- function() hidden\nmake <- function() {{ hidden <- 1; {declaration}; G }}\n"
        );
        let root = arity::parser::parse(&source).cst;
        let model = arity::project::classes::standalone_semantic_model(&root);
        let read = model
            .idents()
            .iter()
            .find(|ident| ident.name == "hidden")
            .unwrap();
        let resolution = model.identifier_resolution(read);
        assert!(
            resolution
                .lexical
                .iter()
                .any(|binding| model.binding(*binding).name == "hidden"),
            "{source}: {resolution:?}"
        );
        assert!(
            resolution.external,
            "the helper also retains its ordinary closure"
        );
    }
}

#[test]
fn mutations_resolve_implementations_in_the_installing_namespace() {
    use arity::project::classes::OopPoint;

    let mut db = IncrementalDatabase::default();
    let owner = db.upsert_file(
        Path::new("/owner/R/a.R"),
        "G <- R6::R6Class()\nimplementation <- function(owner_private) owner_private\n",
    );
    let helper = db.upsert_file(
        Path::new("/helpers/R/a.R"),
        "implementation <- function(imported) imported\n",
    );
    let installer_path = Path::new("/installer/R/a.R");
    let installer = db.upsert_file(
        installer_path,
        "owner::G$set('public', 'read', implementation)\n",
    );
    db.set_workspace_members(
        vec![owner, helper, installer],
        ["/owner", "/helpers", "/installer"].map(Into::into).into(),
    );
    db.refresh_descriptions(["/owner".into(), "/helpers".into(), "/installer".into()]);
    for (root, description) in [
        ("/owner", "Package: owner\n"),
        ("/helpers", "Package: helpers\n"),
        (
            "/installer",
            "Package: installer\nImports: owner, helpers\n",
        ),
    ] {
        db.upsert_description(Path::new(root), description);
    }
    db.set_package_metadata(
        [
            ("/owner", "export(G)\n"),
            ("/helpers", "export(implementation)\n"),
            ("/installer", "importFrom(helpers, implementation)\n"),
        ]
        .map(|(root, namespace)| arity::project::PackageInfo {
            root: root.into(),
            namespace: Some(namespace.into()),
            dir_sources: Default::default(),
            native_routines: Default::default(),
        })
        .into(),
    );
    let index = db.snapshot().class_index();
    let api = index
        .r6_api_at(
            &class(&index, "G"),
            Some(&OopPoint {
                path: installer_path.into(),
                event: usize::MAX,
            }),
        )
        .unwrap();
    assert!(api.public.complete);
    assert_eq!(
        api.public.entries["read"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .name,
        "imported"
    );
}

#[test]
fn nested_generators_do_not_replace_package_bindings() {
    let mut db = IncrementalDatabase::default();
    let root = Path::new("/nested-mutations");
    let definition = db.upsert_file(
        &root.join("R/a.R"),
        "G <- R6::R6Class()\nmake <- function() { G <- R6::R6Class(); G }\n",
    );
    let mutation = db.upsert_file(&root.join("R/b.R"), "G$set('public', 'added', 1)\n");
    db.set_workspace_members(vec![definition, mutation], vec![root.into()]);
    db.set_package_metadata(vec![arity::project::PackageInfo {
        root: root.into(),
        namespace: None,
        dir_sources: Default::default(),
        native_routines: Default::default(),
    }]);
    let index = db.snapshot().class_index();
    for id in index.declarations.keys() {
        let api = index.r6_api(id).unwrap();
        assert_eq!(
            api.public.entries.contains_key("added"),
            id.key.owner.0.is_empty(),
            "{id:?}"
        );
    }
}

#[test]
fn external_method_enclosures_preserve_lexical_owners() {
    let mut db = IncrementalDatabase::default();
    let root = Path::new("/lexical-mutations");
    let definition = db.upsert_file(
        &root.join("R/a.R"),
        "G <- R6::R6Class(private = list(secret = 1))\n",
    );
    let implementation = db.upsert_file(
        &root.join("R/b.R"),
        "implementation <- function() self$value + private$secret\nmake <- function() { implementation <- function() self$value + private$secret; implementation }\n",
    );
    let mutation = db.upsert_file(
        &root.join("R/c.R"),
        "G$set('public', 'read', implementation)\n",
    );
    db.set_workspace_members(
        vec![definition, implementation, mutation],
        vec![root.into()],
    );
    db.set_package_metadata(vec![arity::project::PackageInfo {
        root: root.into(),
        namespace: None,
        dir_sources: Default::default(),
        native_routines: Default::default(),
    }]);
    let model = db.runtime_semantic_model(implementation);
    for name in ["self", "private"] {
        let reads: Vec<_> = model
            .idents()
            .iter()
            .filter(|ident| ident.name == name)
            .collect();
        assert_eq!(reads.len(), 2);
        assert!(!model.identifier_resolution(reads[0]).generated.is_empty());
        assert!(
            model.is_free_read(reads[1]),
            "{name} in the unrelated nested function"
        );
    }
}

#[test]
fn mutator_aliases_capture_the_receiver_at_acquisition() {
    for call in [
        "setter('public', 'added', 1)",
        "copied <- setter; copied('public', 'added', 1)",
        "setter('public', 'first', 1); setter('public', 'added', 2)",
    ] {
        let index = index(&format!(
            "G <- R6::R6Class()\nH <- R6::R6Class()\nAlias <- G\nsetter <- Alias$set\nAlias <- H\n{call}\n",
        ));
        for (binding, added) in [("G", true), ("H", false)] {
            let api = index.r6_api(&class(&index, binding)).unwrap();
            assert!(api.public.complete);
            assert_eq!(
                api.public.entries.contains_key("added"),
                added,
                "{binding}: {call}"
            );
        }
    }
}

#[test]
fn reference_null_removes_inherited_methods_and_restores_own_methods() {
    let source = r#"
Parent <- methods::setRefClass("RemovalParent")
Parent$methods(read = function(parent) parent)
Child <- methods::setRefClass("RemovalChild", contains = "RemovalParent", methods = list(own = function(original) original))
Child$methods(read = NULL, own = function(replaced) replaced)
Child$methods(own = NULL)
"#;
    let index = index(source);
    let api = index.reference_api(&class(&index, "Child")).unwrap();
    assert!(api.instance.complete);
    assert!(!api.instance.entries.contains_key("read"));
    assert_eq!(
        api.instance.entries["own"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .name,
        "replaced"
    );
    assert_eq!(help(&format!("{source}\nChild()$read(|)")), None);
}

#[test]
fn r6_set_resolves_list_fields_before_classifying_the_installation() {
    let source = "G <- R6::R6Class(public = list(read = function(x) x))\nG$set('public', 'data', list())\nG$set('private', 'cache', base::list())\n";
    let index = index(source);
    let api = index.r6_api(&class(&index, "G")).unwrap();
    assert!(api.public.complete && api.private.complete);
    assert_eq!(api.public.entries["data"][0].kind, MemberKind::Field);
    assert_eq!(api.private.entries["cache"][0].kind, MemberKind::Field);
    assert_eq!(
        help(&format!("{source}\nG$new()$read(|)")),
        Some("read(x)".into())
    );
}

#[test]
fn quoted_operations_do_not_change_the_live_class_api() {
    for operation in [
        "expr <- quote(G$set('public', 'added', 1))",
        "expr <- base::quote(G$set('public', 'added', 1))",
        "expr <- expression(G$set('public', 'added', 1))",
        "expr <- substitute(G$set('public', 'added', 1))",
        "expr <- bquote(G$set('public', 'added', 1))",
        "expr <- rlang::expr(G$set('public', 'added', 1))",
        "expr <- quote(bquote(.(G$set('public', 'added', 1))))",
        "expr <- quote(G$public_methods$read <- implementation)",
        "expr <- quote(assign('G', NULL))",
        "expr <- quote(G$lock())",
    ] {
        let source =
            format!("G <- R6::R6Class(public = list(read = function(x) x))\n{operation}\n");
        let index = index(&source);
        let api = index.r6_api(&class(&index, "G")).unwrap();
        assert!(api.public.complete, "{operation}");
        assert!(!api.public.entries.contains_key("added"));
        assert_eq!(
            help(&format!("{source}G$new()$read(|)")),
            Some("read(x)".into()),
            "{operation}"
        );
    }
}

#[test]
fn unquoted_operations_keep_the_class_api_uncertain() {
    for operation in [
        "expr <- bquote(.(G$set('public', 'added', 1)))",
        "expr <- rlang::expr(!!G$set('public', 'added', 1))",
    ] {
        let index = index(&format!("G <- R6::R6Class()\n{operation}\n"));
        let api = index.r6_api(&class(&index, "G")).unwrap();
        assert!(!api.public.complete, "{operation}");
    }
}
