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
fn r6_members_resolve_stable_callable_aliases() {
    let facts = facts(
        "helper <- function(x, scale = 2) x * scale\nalias <- helper\n\
         A <- R6::R6Class('A', public = base::list(read = alias),\n\
             private = base::list(write = helper), active = base::list(value = helper))\n",
    );
    assert_eq!(facts.oop.methods.len(), 3);
    for member in &facts.oop.members {
        assert!(matches!(
            member.kind,
            MemberKind::Method | MemberKind::ActiveBinding
        ));
        assert!(member.uncertainty.is_empty());
        let key = member.method.as_ref().unwrap();
        let method = facts
            .oop
            .methods
            .iter()
            .find(|method| &method.key == key)
            .unwrap();
        let parameters = &method.signature.as_ref().unwrap().parameters;
        assert_eq!(parameters[0].name, "x");
        assert_eq!(parameters[1].name, "scale");
        assert_eq!(parameters[1].default.as_deref(), Some("2"));
    }
}

#[test]
fn r6_unknown_member_values_remain_uncertain() {
    for source in [
        "A <- R6::R6Class('A', public = base::list(read = helper))",
        "helper <- function(x) x\nhelper <- 1\nA <- R6::R6Class('A', public = base::list(read = helper))",
        "if (flag) helper <- function(x) x\nA <- R6::R6Class('A', public = base::list(read = helper))",
        "A <- R6::R6Class('A', public = base::list(read = make_helper()))",
        "A <- R6::R6Class('A', public = base::list(read = helper))\nhelper <- function(x) x",
    ] {
        let facts = facts(source);
        assert!(facts.oop.methods.is_empty(), "{source}");
        assert_eq!(facts.oop.members[0].kind, MemberKind::Unknown, "{source}");
        assert!(
            facts.oop.members[0]
                .uncertainty
                .contains(&OopUncertainty::UnresolvedImplementation),
            "{source}"
        );
    }
    let facts =
        facts("value <- 1\nA <- R6::R6Class('A', public = base::list(x = value, y = NULL))");
    assert!(
        facts
            .oop
            .members
            .iter()
            .all(|member| member.kind == MemberKind::Field && member.uncertainty.is_empty())
    );
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

#[test]
fn s7_registration_resolves_the_replacement_callee_and_matches_its_formals() {
    let facts = facts(
        r#"
g <- S7::new_generic("g", "x")
C <- S7::new_class("C")
S7::method(g, signature = C) <- function(x) x
setter <- S7::`method<-`
setter(value = function(x) x, generic = g, signature = C)
`install<-` <- setter
install(g, C) <- function(x) x
"#,
    );
    assert_eq!(facts.oop.methods.len(), 3);
    for method in &facts.oop.methods {
        assert_eq!(method.name, ClassName::Known("g".into()));
        assert_eq!(
            method.dispatch[0].class,
            DispatchClass::Generator(GeneratorRef::Local(facts.declarations[0].key.clone()))
        );
        assert_eq!(method.signature.as_ref().unwrap().parameters[0].name, "x");
        assert_eq!(
            method.registration,
            Some(FunctionRef::Qualified {
                package: "S7".into(),
                name: "method<-".into(),
            })
        );
        assert!(method.uncertainty.is_empty(), "{method:?}");
    }
}

#[test]
fn getters_and_shadowed_replacement_callees_are_not_registrations() {
    for source in [
        "getter <- S7::method\ngetter(g, C) <- function(x) x",
        "`method<-` <- function(...) NULL\nmethod(g, C) <- function(x) x",
        "method <- S7::method\n`method<-` <- other::`method<-`\nmethod(g, C) <- function(x) x",
        "S7::method(g, C)",
        "other::`method<-`(g, C, function(x) x)",
    ] {
        assert!(facts(source).oop.methods.is_empty(), "{source}");
    }
    let facts = facts("method <- function(...) NULL\nmethod(g, C) <- function(x) x");
    assert_eq!(
        facts.oop.methods.len(),
        1,
        "only the setter binding matters"
    );
    assert!(
        facts.oop.methods[0]
            .uncertainty
            .contains(&OopUncertainty::UnresolvedCallee)
    );
}

#[test]
fn unresolved_aliases_retain_all_supported_declaration_candidates() {
    let facts = facts(
        r#"
declare <- methods::setGeneric
if (flag) declare <- S7::new_generic
declare("g", function(x) x)
register <- methods::setMethod
if (flag) register <- base::registerS3method
register("g", "C", function(x) x)
if (flag) `install<-` <- S7::`method<-`
install(g, C) <- function(x) x
"#,
    );
    assert_eq!(facts.oop.generics.len(), 2);
    assert_eq!(facts.oop.methods.len(), 3);
    assert!(facts.oop.generic_bindings.is_empty());
    for issues in facts
        .oop
        .generics
        .iter()
        .map(|d| &d.uncertainty)
        .chain(facts.oop.methods.iter().map(|d| &d.uncertainty))
    {
        assert!(
            issues.contains(&OopUncertainty::UnresolvedCallee),
            "{issues:?}"
        );
    }
}

#[test]
fn replacement_registration_matches_the_outer_wrapper_before_forwarding() {
    let facts = facts(
        r#"
methods::setReplaceMethod("value", "C", function(object, value) object,
    valueClass = "C", sealed = TRUE, where = target)
methods::setReplaceMethod(f = "value", signature = "C", definition = function(object, value) object)
methods::setReplaceMethod("value", "C", function(object, value) object, "C", TRUE)
"#,
    );
    assert_eq!(facts.oop.methods.len(), 3);
    for method in &facts.oop.methods {
        assert_eq!(method.name, ClassName::Known("value<-".into()));
        assert_eq!(method.dispatch[0].class, DispatchClass::Formal("C".into()));
        assert!(
            !method
                .uncertainty
                .contains(&OopUncertainty::InvalidArguments)
        );
    }
    assert!(
        facts.oop.methods[0]
            .uncertainty
            .contains(&OopUncertainty::CustomEnvironment)
    );
    assert!(
        !facts.oop.methods[2]
            .uncertainty
            .contains(&OopUncertainty::CustomEnvironment)
    );
}

#[test]
fn replacement_generic_objects_resolve_the_setter_independently() {
    for registration in ["gname", "alias", "'value'"] {
        let facts = facts(&format!(
            "gname <- methods::setGeneric('value', function(object) object)\n\
             methods::setGeneric('value<-', function(object, value) object)\n\
             alias <- gname\n\
             methods::setReplaceMethod({registration}, 'C', function(object, value) object)\n"
        ));
        let reference = facts.oop.methods[0].generic.as_ref().unwrap();
        assert_eq!(reference.name, ClassName::Known("value<-".into()));
        assert_eq!(
            facts.oop.generic_candidates(reference),
            vec![&facts.oop.generics[1]]
        );
        assert!(
            facts
                .oop
                .methods_for_generic(&facts.oop.generics[0].key)
                .is_empty()
        );
        assert_eq!(
            facts
                .oop
                .methods_for_generic(&facts.oop.generics[1].key)
                .len(),
            1
        );
    }
    let facts = facts(
        "gname <- methods::setGeneric('value', function(object) object)\n\
         f <- function() {\n\
             methods::setGeneric('value<-', function(object, value) object)\n\
             methods::setReplaceMethod(gname, 'C', function(object, value) object)\n\
         }\n",
    );
    assert_eq!(
        facts
            .oop
            .generic_candidates(facts.oop.methods[0].generic.as_ref().unwrap()),
        vec![&facts.oop.generics[1]]
    );
}

#[test]
fn s4_registration_accepts_proven_generic_objects_including_backticked_names() {
    let facts = facts(
        r#"
methods::setGeneric("value<-", function(object, value) standardGeneric("value<-"))
alias <- `value<-`
methods::setMethod(alias, "C", function(object, value) object)
methods::setMethod(`value<-`, "C", function(object, value) object)
"#,
    );
    assert_eq!(facts.oop.methods.len(), 2);
    for method in &facts.oop.methods {
        assert_eq!(method.name, ClassName::Known("value<-".into()));
        assert_eq!(
            facts
                .oop
                .generic_candidates(method.generic.as_ref().unwrap()),
            vec![&facts.oop.generics[0]]
        );
        assert!(!method.uncertainty.contains(&OopUncertainty::DynamicName));
    }
    let shadowed = self::facts(
        "g <- methods::setGeneric('g', function(x) x)\nf <- function(g) methods::setMethod(g, 'C', function(x) x)",
    );
    assert_eq!(shadowed.oop.methods[0].name, ClassName::Unknown);
}

#[test]
fn known_callee_aliases_preserve_operator_and_replacement_names() {
    let facts = facts(
        r#"
define <- methods::setGeneric
install <- methods::setMethod
define(name = "[<-", def = function(x, i, value) standardGeneric("[<-"))
install(definition = function(x, i, value) x, signature = "C", f = "[<-")
register <- base::registerS3method
register(method = function(e1, e2) e1, class = "C", genname = "+")
`+.C` <- function(e1, e2) e1
register("+", "C", `+.C`)
"#,
    );
    assert_eq!(facts.oop.generics.len(), 1);
    assert_eq!(facts.oop.methods.len(), 3);
    assert_eq!(facts.oop.methods[0].name, ClassName::Known("[<-".into()));
    assert_eq!(facts.oop.methods[1].name, ClassName::Known("+".into()));
    assert_eq!(
        facts.oop.methods[2].signature.as_ref().unwrap().parameters[0].name,
        "e1"
    );
    assert!(facts.oop.methods.iter().all(|d| d.uncertainty.is_empty()));
}

#[test]
fn uncertain_class_origins_do_not_create_definite_members() {
    let facts = facts(
        r#"
make <- R6::R6Class
if (flag) make <- other::R6Class
A <- make("A", public = base::list(read = function(x) x))
"#,
    );
    assert_eq!(facts.oop.members.len(), 1);
    assert!(
        facts.oop.members[0]
            .uncertainty
            .contains(&OopUncertainty::UnresolvedCallee)
    );
    assert!(
        facts.oop.methods[0]
            .uncertainty
            .contains(&OopUncertainty::UnresolvedCallee)
    );
}

#[test]
fn conditional_generic_objects_and_unknown_aliases_do_not_choose_declarations() {
    let facts = facts(
        r#"
if (flag) g <- S7::new_generic("g", "x")
alias <- g
S7::method(alias, C) <- function(x) x
"#,
    );
    assert!(facts.oop.generic_bindings.is_empty());
    assert!(
        facts.oop.methods[0]
            .generic
            .as_ref()
            .unwrap()
            .local
            .is_none()
    );
    assert!(
        facts
            .oop
            .generic_candidates(facts.oop.methods[0].generic.as_ref().unwrap())
            .is_empty()
    );
    assert!(
        facts.oop.methods[0]
            .uncertainty
            .contains(&OopUncertainty::UnresolvedAlias)
    );
}

#[test]
fn quoted_reassignments_cannot_prove_callee_aliases() {
    let facts = facts(
        "register <- methods::setMethod\n`register` <- function(...) NULL\nregister('g', 'C', function(x) x)",
    );
    assert_eq!(facts.oop.methods.len(), 1);
    assert!(
        facts.oop.methods[0]
            .uncertainty
            .contains(&OopUncertainty::UnresolvedCallee)
    );
}

#[test]
fn parenthesized_and_conditional_alias_targets_remain_lossless_and_total() {
    let facts = facts(
        r#"
(R6::R6Class)("A")
declare <- (methods::setGeneric)
declare("g", function(x) x)
register <- if (flag) methods::setMethod else base::registerS3method
register("g", "C", function(x) x)
"#,
    );
    assert_eq!(facts.declarations.len(), 1);
    assert_eq!(facts.oop.generics.len(), 1);
    assert_eq!(facts.oop.methods.len(), 2);
    assert!(
        facts
            .oop
            .methods
            .iter()
            .all(|def| def.uncertainty.contains(&OopUncertainty::UnresolvedCallee))
    );
}

#[test]
fn invalid_registration_arguments_survive_as_incomplete_facts() {
    for source in [
        "methods::setMethod(f = 'g', f = 'h', signature = 'C', definition = function(x) x)",
        "methods::setReplaceMethod('g', 'C', function(x) x, w = target)",
        "S7::method(g, class = C) <- function(x) x",
        "S7::method(g, C, value = function(x) x) <- function(x) x",
        "base::registerS3method('g', 'C', function(x) x, ...)",
    ] {
        let facts = facts(source);
        assert_eq!(facts.oop.methods.len(), 1, "{source}");
        let method = &facts.oop.methods[0];
        assert!(
            method
                .uncertainty
                .contains(&OopUncertainty::InvalidArguments),
            "{source}: {method:?}"
        );
        assert_eq!(method.name, ClassName::Unknown);
    }
}

#[test]
fn defining_and_registration_names_require_supported_origins() {
    for (name, arguments) in [
        ("R6Class", "'C'"),
        ("setClass", "'C'"),
        ("setRefClass", "'C'"),
        ("new_class", "'C'"),
        ("setGeneric", "'g', function(x) x"),
        ("new_generic", "'g', 'x'"),
        ("setMethod", "'g', 'C', function(x) x"),
        ("setReplaceMethod", "'g', 'C', function(x) x"),
        ("registerS3method", "'g', 'C', function(x) x"),
        ("method<-", "g, C, function(x) x"),
    ] {
        for source in [
            format!("other::`{name}`({arguments})"),
            format!("f <- function(`{name}`) `{name}`({arguments})"),
            format!("`{name}` <- function(...) NULL\n`{name}`({arguments})"),
        ] {
            let facts = facts(&source);
            assert!(facts.declarations.is_empty(), "{source}");
            assert!(facts.oop.generics.is_empty(), "{source}");
            assert!(facts.oop.methods.is_empty(), "{source}");
        }
    }
    assert!(
        facts("f <- function(UseMethod) UseMethod('f')")
            .oop
            .generics
            .is_empty()
    );
}

#[test]
fn alias_registration_sites_and_facts_survive_formatting_and_reparse() {
    let source = "declare <- methods::setGeneric\ndeclare('g',function(x) x)\ninstall <- methods::setMethod\ninstall(definition=function(x) x,signature='C',f='g')\ng7 <- S7::new_generic('g7','x')\n`install7<-` <- S7::`method<-`\ninstall7(g7,signature=C) <- function(x) x\n";
    let formatted = arity::formatter::format(source).unwrap();
    assert_eq!(arity::formatter::format(&formatted).unwrap(), formatted);
    assert_eq!(facts(source), facts(&formatted));
    let parsed = parse(source);
    let edit = arity::parser::Edit {
        range: 7..7,
        insert: "  ".into(),
    };
    let shifted = edit.apply(source);
    let incremental =
        arity::parser::reparse(&parsed.cst, source, &parsed.diagnostics, &edit).unwrap();
    let fresh = parse(&shifted);
    assert_eq!(incremental.green, fresh.cst.green().to_owned());
    assert_eq!(incremental.diagnostics, fresh.diagnostics);
    let before = oop_sites(&SemanticModel::build(&parsed.cst), &parsed.cst);
    let incremental_root = arity::syntax::SyntaxNode::new_root(incremental.green);
    let after = oop_sites(&SemanticModel::build(&incremental_root), &incremental_root);
    assert_eq!(facts(source), facts(&shifted));
    assert_eq!(before.len(), after.len());
    for (before, after) in before.iter().zip(after) {
        assert_eq!(before.target, after.target);
        assert_eq!(before.role, after.role);
        assert_eq!(&source[before.range], &shifted[after.range]);
    }
}
