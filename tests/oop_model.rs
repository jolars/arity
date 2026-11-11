use arity::parser::parse;
use arity::project::classes::*;
use arity::semantic::SemanticModel;

fn facts(source: &str) -> FileClassFacts {
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(parsed.cst.text().to_string(), source);
    file_class_defs(&SemanticModel::build(&parsed.cst), &parsed.cst)
}

#[test]
fn members_keep_class_identity_visibility_and_callable_signatures() {
    let source = r#"
Generator <- R6::R6Class("Display", public = list(
    value = NULL, read = function(scale = 1) self$value * scale),
    private = list(value = 0), active = list(doubled = function() self$value * 2))
Alias <- Generator
Other <- R6::R6Class("Display", public = list(value = 2))
Ref <- methods::setRefClass("Display", fields = list(value = "numeric"),
    methods = list(read = function(scale = 2) value * scale))
Formal <- methods::setClass("Display", slots = c(value = "numeric"))
"#;
    let facts = facts(source);
    assert_eq!(facts.declarations.len(), 4);
    assert_eq!(
        facts
            .generators
            .iter()
            .find(|g| g.name == "Alias")
            .unwrap()
            .target,
        facts.declarations[0].key
    );
    let members = &facts.oop.members;
    assert_eq!(members.len(), 8);
    assert_eq!(members[0].class, facts.declarations[0].key);
    assert_eq!(members[2].visibility, MemberVisibility::Private);
    assert_eq!(members[3].kind, MemberKind::ActiveBinding);
    assert_ne!(members[0].class, members[4].class);
    let method = &facts.oop.methods[0];
    assert_eq!(
        method.signature.as_ref().unwrap().parameters[0].name,
        "scale"
    );
    assert_eq!(
        method.signature.as_ref().unwrap().parameters[0]
            .default
            .as_deref(),
        Some("1")
    );
    assert_eq!(facts.oop.methods.len(), 3);
}

#[test]
fn generics_and_duplicate_methods_keep_identity_and_dispatch() {
    let facts = facts(
        r#"
methods::setGeneric("combine", function(x, y) standardGeneric("combine"))
methods::setGeneric("combine", function(x, y) standardGeneric("combine"))
methods::setMethod("combine", c(x = "A", y = "B"), function(x, y) 1)
methods::setMethod("combine", c(x = "A", y = "B"), function(x, y) 2)
methods::setReplaceMethod("value", "A", function(object, value) object)
"#,
    );
    assert_eq!(facts.oop.generics.len(), 2);
    assert_ne!(facts.oop.generics[0].key, facts.oop.generics[1].key);
    assert_eq!(facts.oop.methods.len(), 3);
    assert_ne!(facts.oop.methods[0].key, facts.oop.methods[1].key);
    assert_eq!(facts.oop.methods[0].dispatch.len(), 2);
    assert_eq!(
        facts.oop.methods[0].dispatch[0].argument.as_deref(),
        Some("x")
    );
    let generic = facts.oop.methods[0].generic.as_ref().unwrap();
    assert_eq!(facts.oop.generic_candidates(generic).len(), 2);
    assert_eq!(
        facts
            .oop
            .methods_for_generic(&facts.oop.generics[0].key)
            .len(),
        2
    );
    assert_eq!(
        facts.oop.methods[2].generic.as_ref().unwrap().name,
        ClassName::Known("value<-".into())
    );
}

#[test]
fn s3_and_s7_share_generic_method_and_class_reference_facts() {
    let facts = facts(
        r#"
describe <- function(x, ...) UseMethod("describe")
describe.point <- function(x, ...) x$value
`+.point` <- function(e1, e2) e1
base::registerS3method("+", "point", `+.point`)
Point <- S7::new_class("Point", properties = list(value = S7::class_numeric))
Alias <- Point
describe7 <- S7::new_generic("describe", c("x", "y"))
S7::method(describe7, list(Alias, Point)) <- function(x, y) x@value
"#,
    );
    assert_eq!(facts.oop.generics.len(), 2);
    assert_eq!(facts.oop.methods.len(), 3);
    assert_eq!(facts.declarations[0].system(), ClassSystem::S7);
    assert_eq!(facts.oop.members[0].kind, MemberKind::Property);
    let method = facts
        .oop
        .methods
        .iter()
        .find(|method| method.system == OopSystem::S7)
        .unwrap();
    assert_eq!(method.system, OopSystem::S7);
    assert_eq!(method.dispatch.len(), 2);
    assert_eq!(
        method.dispatch[0].class,
        DispatchClass::Generator(GeneratorRef::Local(facts.declarations[0].key.clone()))
    );
    assert_eq!(
        facts
            .oop
            .generic_candidates(method.generic.as_ref().unwrap())
            .len(),
        1
    );
}

#[test]
fn sites_recover_current_spans_and_keep_receiver_ambiguity() {
    let source = r#"A <- R6::R6Class("A", public = list(read = function() self$value))
B <- R6::R6Class("B", public = list(value = 2))
Alias <- A
Alias$new()
"#;
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    let sites = oop_sites(&model, &parsed.cst);
    let value = sites
        .iter()
        .find(|site| &source[site.range] == "value" && site.role == OopSiteRole::Use)
        .unwrap();
    assert!(
        matches!(&value.target, OopTarget::Member { receiver: MemberReceiver::Implicit { class, .. }, .. }
        if class == &facts(source).declarations[0].key)
    );
    let shifted = format!("# shifted\n{source}");
    let root = parse(&shifted).cst;
    let shifted_sites = oop_sites(&SemanticModel::build(&root), &root);
    assert_eq!(sites.len(), shifted_sites.len());
    for (before, after) in sites.iter().zip(shifted_sites) {
        assert_eq!(before.target, after.target);
        assert_eq!(before.role, after.role);
        assert_eq!(&source[before.range], &shifted[after.range]);
    }
}

#[test]
fn shared_projection_ignores_bodies_but_tracks_members_and_signatures() {
    let source = "A <- R6::R6Class('A', public = list(read = function(x = 1) x))";
    let before = facts(source);
    assert_eq!(
        before,
        facts(&source.replace(") x", ") { print(x); x + 2 }"))
    );
    assert_ne!(before, facts(&source.replace("read =", "write =")));
    assert_ne!(before, facts(&source.replace("x = 1", "x = 2")));
}

#[test]
fn lexical_owners_and_generic_aliases_do_not_merge_same_named_generics() {
    let facts = facts(
        r#"
first <- S7::new_generic("display", "x")
second <- S7::new_generic("display", "x")
alias <- first
S7::method(alias, UnknownClass) <- function(x) 1
factory <- function() {
    local <- S7::new_generic("display", "x")
    S7::method(local, UnknownClass) <- function(x) 2
}
"#,
    );
    assert_eq!(facts.oop.generics.len(), 3);
    assert_eq!(facts.oop.methods.len(), 2);
    for (method, generic) in facts
        .oop
        .methods
        .iter()
        .zip([&facts.oop.generics[0], &facts.oop.generics[2]])
    {
        assert_eq!(
            facts
                .oop
                .generic_candidates(method.generic.as_ref().unwrap()),
            vec![generic]
        );
    }
    assert_ne!(
        facts.oop.generics[0].key.owner,
        facts.oop.generics[2].key.owner
    );
}

#[test]
fn ambiguous_members_and_incomplete_declarations_survive_projection() {
    let facts = facts(
        r#"
A <- R6::R6Class("A", public = list(value = 1, value = 2))
B <- R6::R6Class("A", public = dynamic_members())
if (flag) methods::setMethod(computed_name(), signature = classes,
    definition = impl, where = environment())
methods::setMethod("read", "A", function(x) 1)
methods::setMethod("read", "A", function(x) 2)
"#,
    );
    assert_eq!(
        facts
            .oop
            .member_candidates(&facts.declarations[0].key, "value")
            .len(),
        2
    );
    assert_eq!(
        facts
            .oop
            .member_candidates(&facts.declarations[1].key, "value")
            .len(),
        0
    );
    assert!(
        facts.oop.member_groups[1]
            .uncertainty
            .contains(&OopUncertainty::DynamicMembers)
    );
    assert_eq!(facts.oop.methods.len(), 3);
    let issues = &facts.oop.methods[0].uncertainty;
    for issue in [
        OopUncertainty::Conditional,
        OopUncertainty::CustomEnvironment,
        OopUncertainty::DynamicName,
        OopUncertainty::DynamicSignature,
        OopUncertainty::UnresolvedImplementation,
    ] {
        assert!(issues.contains(&issue), "missing {issue:?}");
    }
    assert_ne!(facts.oop.methods[1].key, facts.oop.methods[2].key);
    let generic = self::facts("g <- S7::new_generic('g', c('x', computed()))");
    assert_eq!(
        generic.oop.generics[0].dispatch_arguments,
        vec![ClassName::Known("x".into()), ClassName::Unknown]
    );
    assert!(
        generic.oop.generics[0]
            .uncertainty
            .contains(&OopUncertainty::DynamicSignature)
    );
}

#[test]
fn shadowed_factories_and_list_lookalikes_create_no_definite_members() {
    let facts = facts(
        r#"
setGeneric <- function(...) NULL
setGeneric("fake", function(x) x)
setMethod <- function(...) NULL
setMethod("fake", "X", function(x) x)
list <- function(...) NULL
A <- R6::R6Class("A", public = list(phantom = 1))
"#,
    );
    assert!(facts.oop.generics.is_empty());
    assert!(facts.oop.methods.is_empty());
    assert!(facts.oop.members.is_empty());
    assert!(
        facts.oop.member_groups[0]
            .uncertainty
            .contains(&OopUncertainty::DynamicMembers)
    );
}

#[test]
fn class_uses_keep_s3_tags_separate_from_formal_classes_and_generators() {
    let source = r#"
structure(list(value = 1), class = c("point", "base"))
class(x) <- "point"
methods::new("point", value = 1)
Point <- S7::new_class("point", properties = list(value = other::Value))
"#;
    let root = parse(source).cst;
    let model = SemanticModel::build(&root);
    let sites = oop_sites(&model, &root);
    let s3 = sites
        .iter()
        .filter(|site| {
            matches!(
                &site.target,
                OopTarget::DispatchClass {
                    system: OopSystem::S3,
                    ..
                }
            )
        })
        .count();
    assert_eq!(s3, 3);
    assert!(sites.iter().any(|site| matches!(&site.target,
        OopTarget::ClassReference(ParentRef::Formal(name)) if name == "point")));
    assert_eq!(
        facts(source).oop.members[0].declared_type,
        Some(DispatchClass::Generator(GeneratorRef::Qualified {
            package: "other".into(),
            name: "Value".into(),
        }))
    );
}

#[test]
fn ambiguous_class_factories_do_not_choose_a_dispatch_class() {
    let facts = facts(
        r#"
make <- R6::R6Class
if (flag) make <- S7::new_class
C <- make("C")
generic <- S7::new_generic("generic", "x")
S7::method(generic, C) <- function(x) x
"#,
    );
    assert_eq!(facts.declarations.len(), 2);
    assert!(facts.generators.is_empty());
    assert_eq!(
        facts.oop.methods[0].dispatch[0].class,
        DispatchClass::Generator(GeneratorRef::Unknown)
    );
}
