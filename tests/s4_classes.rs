use arity::incremental::IncrementalDatabase;
use arity::lsp::compute_completions;
use arity::parser::parse;
use arity::project::classes::*;
use arity::rindex::provider::IndexedProvider;
use arity::semantic::SemanticModel;
use lsp_types::{CompletionItem, CompletionResponse};
use std::path::Path;

fn facts(source: &str) -> FileClassFacts {
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(parsed.cst.text().to_string(), source);
    file_class_defs(&SemanticModel::build(&parsed.cst), &parsed.cst)
}

fn completion(marked: &str) -> Vec<CompletionItem> {
    let offset = marked.find('|').unwrap();
    match compute_completions(
        &marked.replacen('|', "", 1),
        offset,
        &IndexedProvider::empty(),
    )
    .unwrap()
    {
        CompletionResponse::List(list) => list.items,
        CompletionResponse::Array(items) => items,
    }
}

fn index(source: &str) -> ClassIndex {
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(parsed.cst.text().to_string(), source);
    let model = SemanticModel::build(&parsed.cst);
    IncrementalDatabase::default().snapshot().class_index_in(
        Path::new("/s4.R"),
        &file_class_defs(&model, &parsed.cst),
        &parsed.cst,
        &model,
    )
}

fn named(index: &ClassIndex, name: &str) -> ClassId {
    let candidates = index
        .declarations
        .iter()
        .filter(|(_, class)| class.declaration.name == ClassName::Known(name.into()))
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    let [id] = candidates.as_slice() else {
        panic!("{name}: {candidates:?}");
    };
    id.clone()
}

#[test]
fn legacy_representation_and_untyped_slots_are_declarations() {
    let facts = facts(
        "methods::setClass('Base', slots = c(value = 'numeric'))\n\
         Generator <- methods::setClass('Legacy', methods::representation('Base', label = 'character'))\n\
         methods::setClass('Untyped', slots = c('first', 'second'))\n",
    );
    let legacy = &facts.declarations[1];
    assert_eq!(legacy.binding.as_deref(), Some("Generator"));
    assert_eq!(
        legacy.parents,
        vec![arity::project::classes::ParentRef::Formal("Base".into())]
    );
    for (name, expected) in [
        ("label", DispatchClass::Formal("character".into())),
        ("first", DispatchClass::Any),
        ("second", DispatchClass::Any),
    ] {
        let member = facts
            .oop
            .members
            .iter()
            .find(|member| member.name == ClassName::Known(name.into()))
            .expect(name);
        assert_eq!(member.kind, MemberKind::Slot);
        assert_eq!(member.declared_type.as_ref(), Some(&expected));
    }
}

#[test]
fn old_class_metadata_does_not_create_a_generator() {
    let facts = facts(
        "Constructor <- methods::setClass('Formal', slots = c(x = 'numeric'))\n\
         Definition <- methods::setClassUnion('Either', c('Formal', 'NULL'))\n\
         Result <- methods::setOldClass(c('child', 'parent'))\n",
    );
    for name in ["Formal", "Either", "child", "parent"] {
        assert!(
            facts
                .declarations
                .iter()
                .any(|class| class.name == ClassName::Known(name.into())),
            "{name}"
        );
    }
    assert_eq!(
        facts
            .generators
            .iter()
            .map(|binding| binding.name.as_str())
            .collect::<Vec<_>>(),
        ["Constructor", "Definition"]
    );
}

#[test]
fn constructors_and_new_expose_inherited_slots() {
    let classes = "Maker <- methods::setClass('Display', slots = c(value = 'numeric'))\n\
        Alias <- Maker\n\
        methods::setClass('Label', slots = c(label = 'character'))\n\
        Child <- methods::setClass('Child', contains = c('Display', 'Label'), slots = c(extra = 'logical'))\n";
    for expression in [
        "Child()@|",
        "methods::new('Child')@|",
        "x <- Child(); alias <- x; alias@|",
    ] {
        let items = completion(&format!("{classes}\n{expression}"));
        for name in ["value", "label", "extra"] {
            assert!(
                items.iter().any(|item| item.label == name),
                "{expression}: {name}: {items:?}"
            );
        }
    }
    let items = completion(&format!("{classes}\nAlias()@|"));
    assert!(items.iter().any(|item| item.label == "value"));
    assert!(!items.iter().any(|item| item.label == "label"));
}

#[test]
fn slot_values_supply_nested_declared_schemas() {
    let classes = "Inner <- methods::setClass('Inner', slots = c(value = 'numeric'))\n\
        Outer <- methods::setClass('Outer', slots = c(inner = 'Inner'))\n\
        obj <- Outer()\n";
    for expression in [
        "obj@inner@|",
        "methods::slot(obj, 'inner')@|",
        "slot(name = 'inner', object = obj)@|",
        "obj@inner <- Inner(); obj@inner@|",
        "slot(obj, 'inner') <- Inner(); obj@inner@|",
        "methods::`slot<-`(obj, 'inner', value = Inner())@|",
        "base::`@<-`(obj, 'inner', Inner())@|",
        "methods::as(obj, 'Inner')@|",
        "methods::`as<-`(obj, 'Inner', value = Inner())@|",
    ] {
        let items = completion(&format!("{classes}\n{expression}"));
        let expected = if expression.contains("`slot<-`")
            || expression.contains("`@<-`")
            || expression.contains("`as<-`")
        {
            "inner"
        } else {
            "value"
        };
        assert!(
            items.iter().any(|item| item.label == expected),
            "{expression}: {items:?}"
        );
    }
}

#[test]
fn pinned_r_class_metadata_slots_and_prototypes_agree() {
    let source = include_str!("fixtures/s4_classes/input.R");
    let index = index(source);
    for row in include_str!("fixtures/s4_classes/expected.tsv").lines() {
        let cells = row.split('\t').collect::<Vec<_>>();
        match cells.as_slice() {
            ["class", name, virtual_class, sealed] => {
                let id = named(&index, name);
                let metadata = index.s4_class(&id).unwrap();
                assert_eq!(
                    metadata.virtual_class,
                    Some(*virtual_class == "TRUE"),
                    "{row}"
                );
                assert_eq!(metadata.sealed, Some(*sealed == "TRUE"), "{row}");
                let slots = index.s4_slots(&id).unwrap();
                assert!(slots.complete, "{name}: {slots:?}");
                let expected_slots = include_str!("fixtures/s4_classes/expected.tsv")
                    .lines()
                    .filter(|row| row.starts_with(&format!("slot\t{name}\t")))
                    .count();
                assert_eq!(slots.entries.len(), expected_slots, "{name}: {slots:?}");
            }
            ["slot", name, slot, class] => {
                let slots = index.s4_slots(&named(&index, name)).unwrap();
                let candidates = slots
                    .entries
                    .get(*slot)
                    .unwrap_or_else(|| panic!("{row}: {slots:?}"));
                assert!(!candidates.is_empty());
                for candidate in candidates {
                    assert_eq!(
                        candidate.declared_type,
                        if *class == "ANY" {
                            DispatchClass::Any
                        } else {
                            DispatchClass::Formal((*class).into())
                        },
                        "{row}"
                    );
                }
            }
            ["generator", name, value] => {
                let file = index.file(Path::new("/s4.R")).unwrap();
                assert_eq!(
                    file.facts
                        .generators
                        .iter()
                        .any(|binding| binding.name == *name),
                    *value == "TRUE",
                    "{row}"
                );
            }
            ["extends", from, to, "TRUE"] => assert!(
                index
                    .supertypes
                    .get(&named(&index, from))
                    .is_some_and(|parents| parents.contains(&named(&index, to))),
                "{row}"
            ),
            ["prototype", name, slot, value] => {
                let prototype = index.s4_prototype(&named(&index, name)).unwrap();
                assert_eq!(
                    prototype.slots.get(*slot),
                    Some(&S4Default::Literal((*value).into())),
                    "{row}"
                );
            }
            ["constructible", name, "TRUE"] => assert_eq!(
                index.s4_class(&named(&index, name)).unwrap().virtual_class,
                Some(false),
                "{row}"
            ),
            ["implicit", name, slot, "TRUE"] => {
                let prototype = index.s4_prototype(&named(&index, name)).unwrap();
                assert!(!prototype.complete, "{row}: {prototype:?}");
                assert_eq!(
                    prototype.slots.get(*slot),
                    Some(&S4Default::Unknown),
                    "{row}"
                );
            }
            _ => {}
        }
    }
    let any = named(&index, "S4Any");
    assert!(
        !index
            .s4_slots(&any)
            .unwrap()
            .entries
            .contains_key("converted")
    );
    assert!(
        !index
            .supertypes
            .get(&any)
            .is_some_and(|parents| parents.contains(&named(&index, "S4Conditional")))
    );
    let facts = &index.file(Path::new("/s4.R")).unwrap().facts;
    assert!(
        facts
            .s4
            .relations
            .iter()
            .any(|relation| relation.conditional)
    );
    assert!(
        facts
            .s4
            .relations
            .iter()
            .any(|relation| relation.coerce && relation.replace)
    );
    assert_eq!(facts.s4.coercions.len(), 1);
    assert!(facts.s4.coercions[0].coerce);
    assert!(facts.declarations[0].s4.as_ref().unwrap().validity);
    assert!(
        !index
            .supertypes
            .get(&named(&index, "S4OldCopy"))
            .is_some_and(|parents| parents.contains(&named(&index, "S4OldShape")))
    );
}

#[test]
#[ignore = "requires Rscript; checks the pinned S4 observations"]
fn live_r_s4_observations() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/s4_classes");
    let output =
        std::process::Command::new(std::env::var_os("RSCRIPT").unwrap_or_else(|| "Rscript".into()))
            .arg("--vanilla")
            .arg(fixture.join("observe.R"))
            .arg(&fixture)
            .output()
            .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        include_str!("fixtures/s4_classes/expected.tsv")
    );
}

#[test]
fn validity_and_coercion_bodies_are_opaque_to_the_projection() {
    let source = "A <- methods::setClass('A', slots = c(value = 'numeric'), validity = function(object) TRUE)\n\
        methods::setClass('B', slots = c(label = 'character'))\n\
        methods::setIs('A', 'B', coerce = function(from) stop('opaque'))\n\
        methods::setAs('B', 'A', function(from) stop('opaque'))\n";
    assert_eq!(
        facts(source),
        facts(
            &source
                .replace("TRUE", "FALSE")
                .replace("stop('opaque')", "stop('changed')")
        )
    );
}

#[test]
fn uncertain_factories_and_dynamic_metadata_do_not_prove_slot_types() {
    for source in [
        "setClass <- function(...) NULL; A <- setClass('A', slots = c(value = 'numeric')); A()@|",
        "A <- other::setClass('A', slots = c(value = 'numeric')); A()@|",
        "slot <- function(...) NULL; A <- methods::setClass('A', slots = c(inner = 'A')); slot(A(), 'inner')@|",
        "A <- methods::setClass('A', slots = unknown); A()@|",
        "A <- methods::setClass('A', contains = 'VIRTUAL', slots = c(value = 'numeric')); A()@|",
        "A <- methods::setClass('A', slots = c(value = 'numeric')); methods::setClassUnion('Union', 'A'); methods::new('Union')@|",
        "A <- methods::setClass('A', representation = methods::representation(value = 'numeric'), slots = c(other = 'character')); A()@|",
    ] {
        assert!(completion(source).is_empty(), "{source}");
    }
    let index = index("methods::setClass('A', slots = c(x = 'numeric'), contains = 'Unknown')");
    let slots = index.s4_slots(&named(&index, "A")).unwrap();
    assert!(!slots.complete);
    assert!(slots.entries.contains_key("x"));
    let source = "A <- methods::setClass(class_name, slots = c(value = 'numeric')); A()@";
    let CompletionResponse::List(list) =
        compute_completions(source, source.len(), &IndexedProvider::empty()).unwrap()
    else {
        panic!("completion list");
    };
    assert!(list.is_incomplete);
    assert!(list.items.iter().any(|item| item.label == "value"));
}

#[test]
fn old_class_templates_copy_structure_without_inventing_a_superclass() {
    let index = index(
        "methods::setClass('Parent', slots = c(parent = 'numeric'))\nmethods::setClass('Shape', contains = 'Parent', slots = c(own = 'character'))\nmethods::setOldClass('Bridge', S4Class = 'Shape')\n",
    );
    let bridge = named(&index, "Bridge");
    let slots = index.s4_slots(&bridge).unwrap();
    assert!(slots.complete, "{slots:?}");
    assert_eq!(
        slots.entries.keys().map(String::as_str).collect::<Vec<_>>(),
        [".S3Class", "own", "parent"]
    );
    assert!(index.supertypes[&bridge].contains(&named(&index, "Parent")));
    assert!(!index.supertypes[&bridge].contains(&named(&index, "Shape")));
    assert_eq!(index.s4_class(&bridge).unwrap().virtual_class, Some(false));
}

#[test]
fn old_class_ancestors_reuse_registered_identities() {
    let index = index(
        "methods::setOldClass('A')\n\
         methods::setOldClass(c('B', 'A'))\n\
         methods::setOldClass(c('C', 'B', 'A'))\n",
    );
    assert_eq!(index.declarations.len(), 3);
    let a = named(&index, "A");
    let b = named(&index, "B");
    let c = named(&index, "C");
    assert!(index.supertypes[&b].contains(&a));
    assert!(index.supertypes[&c].contains(&b));
    for id in [a, b, c] {
        let slots = index.s4_slots(&id).unwrap();
        assert!(slots.complete, "{id:?}: {slots:?}");
        assert_eq!(slots.entries.len(), 1);
        assert!(slots.entries.contains_key(".S3Class"));
    }
}

#[test]
fn old_class_prototypes_determine_constructibility() {
    for (prototype, virtual_class) in [
        ("", Some(true)),
        (", prototype = NULL", Some(true)),
        (", prototype = list()", Some(false)),
        (
            ", prototype = structure(list(), class = 'Old')",
            Some(false),
        ),
        (", prototype = make_prototype()", None),
        (", prototype = supplied", None),
    ] {
        let source = format!("methods::setOldClass('Old'{prototype})\n");
        let index = index(&source);
        assert_eq!(
            index.s4_class(&named(&index, "Old")).unwrap().virtual_class,
            virtual_class,
            "{source}"
        );
        let items = completion(&format!("{source}methods::new('Old')@|"));
        assert_eq!(
            items.iter().any(|item| item.label == ".S3Class"),
            virtual_class == Some(false),
            "{source}: {items:?}"
        );
    }
    let index = index(
        "methods::setOldClass(c('Child', 'Parent'), prototype = structure(list(), class = c('Child', 'Parent')))\n",
    );
    assert_eq!(
        index
            .s4_class(&named(&index, "Child"))
            .unwrap()
            .virtual_class,
        Some(false)
    );
    assert_eq!(
        index
            .s4_class(&named(&index, "Parent"))
            .unwrap()
            .virtual_class,
        Some(true)
    );
}

#[test]
fn self_template_bridges_preserve_registration_parents() {
    let index = index(
        "methods::setClass('Base', slots = c(base = 'character'))\n\
         methods::setClass('Formal', contains = 'Base', slots = c(x = 'numeric'))\n\
         methods::setOldClass(c('Formal', 'Ancestor'), S4Class = 'Formal')\n",
    );
    let formal = named(&index, "Formal");
    assert!(index.supertypes[&formal].contains(&named(&index, "Ancestor")));
    assert!(index.supertypes[&formal].contains(&named(&index, "Base")));
    let slots = index.s4_slots(&formal).unwrap();
    assert!(slots.complete, "{slots:?}");
    assert_eq!(
        slots.entries.keys().map(String::as_str).collect::<Vec<_>>(),
        [".S3Class", "base", "x"]
    );
}

#[test]
fn old_class_templates_copy_relations_without_copying_relation_slots() {
    let index = index(
        "methods::setClass('Shape', slots = c(x = 'numeric'))\n\
         methods::setClass('Target', slots = c(converted = 'character'))\n\
         methods::setIs('Shape', 'Target', coerce = function(from) methods::new('Target'))\n\
         methods::setClassUnion('Either', 'Shape')\n\
         methods::setOldClass('Bridge', S4Class = 'Shape')\n",
    );
    let bridge = named(&index, "Bridge");
    for parent in ["Target", "Either"] {
        let parent = named(&index, parent);
        assert!(index.supertypes[&bridge].contains(&parent));
        assert!(index.subtypes[&parent].contains(&bridge));
    }
    assert!(!index.supertypes[&bridge].contains(&named(&index, "Shape")));
    let slots = index.s4_slots(&bridge).unwrap();
    assert!(slots.complete, "{slots:?}");
    assert_eq!(
        slots.entries.keys().map(String::as_str).collect::<Vec<_>>(),
        [".S3Class", "x"]
    );
}

#[test]
fn special_builtin_parents_supply_data_slots() {
    for (parent, slot) in [("ANY", ".Data"), ("S4", ".Data"), ("NULL", ".xData")] {
        let source = format!("methods::setClass('Data', contains = '{parent}')\n");
        let index = index(&source);
        let slots = index.s4_slots(&named(&index, "Data")).unwrap();
        assert!(slots.complete, "{parent}: {slots:?}");
        assert_eq!(slots.entries.len(), 1, "{parent}: {slots:?}");
        assert_eq!(
            slots.entries[slot][0].declared_type,
            if parent == "ANY" {
                DispatchClass::Any
            } else {
                DispatchClass::Formal(parent.into())
            }
        );
        assert!(
            completion(&format!("{source}methods::new('Data')@|"))
                .iter()
                .any(|item| item.label == slot),
            "{parent}"
        );
    }
}

#[test]
fn first_parent_owns_prototype_slots_even_with_implicit_defaults() {
    let index = index(
        "methods::setClass('A', slots = c(x = 'numeric'))\n\
         methods::setClass('B', slots = c(x = 'numeric'), prototype = list(x = 2))\n\
         methods::setClass('C', contains = c('A', 'B'))\n\
         methods::setClass('Nested', contains = c('C', 'B'))\n\
         methods::setClass('Reverse', contains = c('B', 'A'))\n\
         methods::setClass('Override', contains = 'C', prototype = list(x = 3))\n",
    );
    for name in ["C", "Nested"] {
        let prototype = index.s4_prototype(&named(&index, name)).unwrap();
        assert!(!prototype.complete, "{name}: {prototype:?}");
        assert_eq!(prototype.slots.get("x"), Some(&S4Default::Unknown));
    }
    for (name, value) in [("Reverse", "2"), ("Override", "3")] {
        let prototype = index.s4_prototype(&named(&index, name)).unwrap();
        assert!(prototype.complete, "{name}: {prototype:?}");
        assert_eq!(prototype.slots["x"], S4Default::Literal(value.into()));
    }
}

#[test]
fn diamonds_deduplicate_declarations_and_conflicting_parents_stay_incomplete() {
    let source = "methods::setClass('Base', slots = c(value = 'numeric'))\nmethods::setClass('Left', contains = 'Base')\nmethods::setClass('Right', contains = 'Base')\nmethods::setClass('Diamond', contains = c('Left', 'Right'))\n";
    let index = index(source);
    let slots = index.s4_slots(&named(&index, "Diamond")).unwrap();
    assert!(slots.complete);
    assert_eq!(slots.entries["value"].len(), 1);
    let index = self::index(
        "methods::setClass('Left', slots = c(value = 'numeric'))\nmethods::setClass('Right', slots = c(value = 'character'))\nmethods::setClass('Conflict', contains = c('Left', 'Right'))\n",
    );
    let slots = index.s4_slots(&named(&index, "Conflict")).unwrap();
    assert!(!slots.complete);
    assert_eq!(slots.entries["value"].len(), 2);
}

#[test]
fn s4_slot_overrides_need_a_proven_narrowing() {
    for (parent_type, child_type, complete) in
        [("ANY", "numeric", true), ("numeric", "character", false)]
    {
        let source = format!(
            "methods::setClass('Parent', slots = c(value = '{parent_type}'))\nmethods::setClass('Child', contains = 'Parent', slots = c(value = '{child_type}'))\n"
        );
        let index = index(&source);
        let slots = index.s4_slots(&named(&index, "Child")).unwrap();
        assert_eq!(slots.complete, complete, "{source}: {slots:?}");
        assert_eq!(
            slots.entries["value"][0].declared_type,
            DispatchClass::Formal(child_type.into())
        );
    }
    for (parents, complete) in [("'Narrow', 'Wide'", true), ("'Wide', 'Narrow'", false)] {
        let source = format!(
            "methods::setClass('Wide', slots = c(value = 'ANY'))\nmethods::setClass('Narrow', slots = c(value = 'numeric'))\nmethods::setClass('Combined', contains = c({parents}))\n"
        );
        let index = index(&source);
        let slots = index.s4_slots(&named(&index, "Combined")).unwrap();
        assert_eq!(slots.complete, complete, "{source}: {slots:?}");
        if complete {
            assert!(
                slots.entries["value"]
                    .iter()
                    .all(|slot| slot.declared_type == DispatchClass::Formal("numeric".into()))
            );
        }
    }
}

#[test]
fn s4_generators_do_not_acquire_reference_class_construction_apis() {
    let source = "Maker <- methods::setClass('A', slots = c(value = 'numeric'))\n";
    for expression in ["Maker$new()@|", "Maker$|", "Maker@|", "Maker()$|"] {
        assert!(
            completion(&format!("{source}{expression}")).is_empty(),
            "{expression}"
        );
    }
}

#[test]
fn resolved_factory_and_accessor_aliases_share_the_slot_schema() {
    let source = "declare <- methods::setClass\nrepr <- methods::representation\ncreate <- methods::new\nfetch <- methods::slot\nwrite <- methods::`slot<-`\nMaker <- declare(Class = 'Inner', representation = repr(value = 'numeric'))\nOuter <- declare(slots = base::list(inner = 'Inner'), Class = 'Outer')\nobj <- Outer()\n";
    for (expression, name) in [
        ("fetch(obj, 'inner')@|", "value"),
        ("create(Class = 'Outer')@|", "inner"),
        ("write(obj, 'inner', value = Maker())@|", "inner"),
        ("base::`@`(obj, 'inner')@|", "value"),
    ] {
        assert!(
            completion(&format!("{source}{expression}"))
                .iter()
                .any(|item| item.label == name),
            "{expression}"
        );
    }
}

#[test]
fn body_local_accessors_resolve_against_the_live_class_index() {
    let source = "methods::setClass('Inner', slots = c(value = 'numeric'))\n\
        methods::setClass('Outer', slots = c(inner = 'Inner'))\n";
    for expression in [
        "methods::slot(obj, 'inner')@|",
        "slot(obj, 'inner')@|",
        "fetch <- methods::slot; fetch(obj, 'inner')@|",
    ] {
        let items = completion(&format!(
            "{source}f <- function() {{ obj <- methods::new('Outer'); {expression} }}"
        ));
        assert!(
            items.iter().any(|item| item.label == "value"),
            "{expression}: {items:?}"
        );
    }
}

#[test]
fn delayed_relationships_keep_lexical_owners_without_changing_global_ancestry() {
    let source = "methods::setClass('A', slots = c(value = 'numeric'))\nmethods::setClass('B', contains = 'VIRTUAL')\ndelayed <- function() { methods::setIs('A', 'B'); methods::setAs('A', 'B', function(from) from); methods::setClassUnion('Union', 'A') }\n";
    let facts = facts(source);
    assert!(
        facts
            .s4
            .relations
            .iter()
            .all(|relation| !relation.owner.0.is_empty())
    );
    assert!(!facts.s4.coercions[0].owner.0.is_empty());
    let index = index(source);
    assert!(
        index
            .supertypes
            .get(&named(&index, "A"))
            .is_none_or(|parents| parents.is_empty())
    );
}

#[test]
fn irregular_old_class_inheritance_stays_incomplete() {
    let index = index("methods::setOldClass(c('Irregular', 'Parent'), test = TRUE)\n");
    let irregular = named(&index, "Irregular");
    assert!(!index.s4_slots(&irregular).unwrap().complete);
    assert!(
        index
            .supertypes
            .get(&irregular)
            .is_none_or(|parents| parents.is_empty())
    );
}
