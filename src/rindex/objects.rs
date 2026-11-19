//! Positive object metadata from installed bindings and method tables.

use std::collections::BTreeSet;

use super::harvest::S3Registration;
use super::lazyload::LazyLoadDb;
use super::rds::{Rkind, Robj};
use super::schema::{
    DispatchParameter, Formal, HelpAssociation, InstalledClass, InstalledGeneric, InstalledMember,
    InstalledMemberKind, InstalledMethod, ObjectMetadata, ObjectName, ObjectSystem, ObjectType,
};

mod serialized;

use serialized::{
    Environment, EnvironmentRecords, binding_reference, closure_namespace, formals, is_class,
    list_field, named_list, object_name, origin, s3_generic, s4_dispatch, s7_dispatch, s7_name,
    s7_type, scalar, strings,
};

pub(super) struct ObjectHarvester<'a> {
    db: Option<&'a LazyLoadDb>,
    package: &'a str,
    namespace_shadows_use_method: bool,
    help: &'a dyn Fn(&str) -> Option<HelpAssociation>,
    environments: EnvironmentRecords<'a>,
    objects: ObjectMetadata,
}

impl<'a> ObjectHarvester<'a> {
    pub(super) fn new(
        db: Option<&'a LazyLoadDb>,
        package: &'a str,
        imports_use_method: bool,
        help: &'a dyn Fn(&str) -> Option<HelpAssociation>,
    ) -> Self {
        Self {
            db,
            package,
            namespace_shadows_use_method: package != "base"
                && (imports_use_method || db.is_some_and(|db| db.contains("UseMethod"))),
            help,
            environments: EnvironmentRecords::new(db),
            objects: ObjectMetadata::default(),
        }
    }

    pub(super) fn observe(&mut self, binding: &str, value: &Robj) {
        if binding.starts_with(".__C__") {
            self.formal_class(binding, value);
        } else if binding.starts_with(".__T__") {
            if let Some(table) = self.environments.resolve(value) {
                for (entry, method) in table.bindings() {
                    self.s4_method(binding, entry, method);
                }
            }
        } else if binding == ".__S3MethodsTable__." {
            if let Some(table) = self.environments.resolve(value)
                && let Some(records) = table.attribute("S7methods").and_then(Robj::as_list)
            {
                for (index, method) in records.iter().enumerate() {
                    self.s7_external_method(binding, index, method);
                }
            }
        } else if !matches!(value.kind, Rkind::Closure { .. }) {
            if let Some(generator) = self.environments.resolve(value)
                && generator.is_class("R6ClassGenerator")
                && let Some(class) = self.r6_class(binding, &generator)
            {
                self.objects.classes.push(class);
            }
        } else if is_class(value, "refObjectGenerator") {
            // The class representation owns the members. Associate its
            // separately serialized generator without duplicating the class.
            if let Some(identity) = value.attr("className").and_then(object_name) {
                self.reference_generator(binding, identity, value);
            }
        } else if is_class(value, "S7_class") {
            if let Some(class) = self.s7_class(binding, value) {
                self.objects.classes.push(class);
            }
        } else if is_class(value, "S7_generic") {
            self.generic(binding, value, ObjectSystem::S7, "name", "dispatch_args");
            if let Some(methods) = value
                .attr("methods")
                .and_then(|m| self.environments.resolve(m))
            {
                // A serialized table can reuse subtables or contain cycles.
                // Bound the total number of entries inspected across all paths.
                let mut remaining = 100_000;
                self.s7_methods(binding, &methods, "", 0, &mut remaining);
            }
        } else if is_class(value, "standardGeneric") || is_class(value, "nonstandardGeneric") {
            self.generic(binding, value, ObjectSystem::S4, "generic", "signature");
        } else if let Some(mut generic) = s3_generic(
            binding,
            value,
            &mut self.environments,
            self.package,
            self.namespace_shadows_use_method,
        ) {
            generic.help = (self.help)(binding);
            self.objects.generics.push(generic);
        }
    }

    pub(super) fn finish(
        mut self,
        exports: &BTreeSet<String>,
        registrations: Vec<S3Registration>,
    ) -> ObjectMetadata {
        if let Some(db) = self.db {
            for name in db.names().filter(|name| !exports.contains(*name)) {
                if let Ok(value) = db.fetch(name) {
                    self.observe(name, &value);
                }
            }
        }
        for registration in registrations {
            let method = registration.method;
            let alias = format!("{}.{}", registration.generic, registration.class);
            self.objects.methods.push(InstalledMethod {
                system: ObjectSystem::S3,
                generic: ObjectName {
                    name: registration.generic.into(),
                    package: registration.package.map(Into::into),
                },
                origin: origin(&method, None),
                dispatch: Some(vec![DispatchParameter {
                    argument: None,
                    class: Some(ObjectType::Named(ObjectName {
                        name: registration.class.into(),
                        package: None,
                    })),
                }]),
                target_dispatch: None,
                conditional: registration.conditional,
                formals: self
                    .db
                    .and_then(|db| db.fetch(&method).ok())
                    .as_ref()
                    .and_then(formals),
                help: (self.help)(&method).or_else(|| (self.help)(&alias)),
            });
        }
        self.objects
    }

    fn generic(
        &mut self,
        binding: &str,
        value: &Robj,
        system: ObjectSystem,
        name: &str,
        signature: &str,
    ) {
        let Some(mut identity) = value.attr(name).and_then(object_name) else {
            return;
        };
        if identity.package.is_none() {
            identity.package = value
                .attr("package")
                .and_then(scalar)
                .map(Into::into)
                .or_else(|| closure_namespace(value));
        }
        self.objects.generics.push(InstalledGeneric {
            system,
            identity,
            origin: origin(binding, None),
            dispatch_arguments: value.attr(signature).and_then(strings),
            formals: formals(value),
            help: (self.help)(binding),
        });
    }

    fn r6_class(&mut self, binding: &str, generator: &Environment) -> Option<InstalledClass> {
        let name = generator.get("classname").and_then(scalar)?;
        let package = generator.get("parent_env").and_then(|env| {
            let Rkind::Namespace(_) = env.kind else {
                return None;
            };
            env.as_str().filter(|name| !name.is_empty()).map(Into::into)
        });
        let groups = [
            ("public_fields", InstalledMemberKind::Field, false),
            ("private_fields", InstalledMemberKind::Field, true),
            ("public_methods", InstalledMemberKind::Method, false),
            ("private_methods", InstalledMemberKind::Method, true),
            ("active", InstalledMemberKind::ActiveBinding, false),
        ];
        let mut members = Vec::new();
        for (group, kind, private) in groups {
            for (member, value) in generator.get(group).into_iter().flat_map(named_list) {
                members.push(self.member(name, member, kind, private, None, formals(value)));
            }
        }
        // The initializer defines the user-facing constructor signature.
        let constructor = members
            .iter()
            .find(|member| member.name == "initialize" && !member.private)
            .and_then(|member| member.formals.clone());
        let inherit = generator.get("inherit");
        let parent_generator = inherit.and_then(binding_reference);
        Some(InstalledClass {
            system: ObjectSystem::R6,
            identity: ObjectName {
                name: name.into(),
                package,
            },
            origin: origin(binding, None),
            generators: Vec::new(),
            parents: Vec::new(),
            parent_known: parent_generator.is_some()
                || inherit.is_some_and(|value| matches!(value.kind, Rkind::Null)),
            parent_generator,
            portable: generator.get("portable").and_then(logical),
            cloneable: generator.get("cloneable").and_then(logical),
            virtual_class: None,
            exported_class: false,
            constructor,
            constructor_returns_instance: true,
            members,
            help: (self.help)(binding).or_else(|| (self.help)(name)),
        })
    }

    fn reference_generator(&mut self, binding: &str, identity: ObjectName, value: &Robj) {
        let generator = origin(binding, None);
        let constructor = formals(value);
        let help = (self.help)(binding);
        if let Some(class) = self
            .objects
            .classes
            .iter_mut()
            .find(|class| class.system == ObjectSystem::Reference && class.identity == identity)
        {
            class.generators.push(generator);
            class.constructor = class.constructor.take().or(constructor);
            class.help = class.help.take().or(help);
            return;
        }
        let definition = value
            .attr("generator")
            .and_then(|slot| slot.attr(".xData"))
            .and_then(|env| self.environments.resolve(env))
            .and_then(|env| env.get("def").cloned());
        let mut class = definition
            .as_ref()
            .and_then(|def| self.class_representation(binding, def))
            .filter(|class| class.system == ObjectSystem::Reference && class.identity == identity)
            .unwrap_or_else(|| InstalledClass {
                system: ObjectSystem::Reference,
                identity,
                origin: origin(binding, None),
                parents: Vec::new(),
                parent_generator: None,
                parent_known: false,
                portable: None,
                cloneable: None,
                virtual_class: None,
                exported_class: false,
                generators: Vec::new(),
                constructor: None,
                constructor_returns_instance: true,
                members: Vec::new(),
                help: None,
            });
        class.generators.push(generator);
        class.constructor = constructor;
        class.help = help.or(class.help);
        self.objects.classes.push(class);
    }

    fn formal_class(&mut self, binding: &str, value: &Robj) {
        let Some(mut class) = self.class_representation(binding, value) else {
            return;
        };
        if let Some(existing) = self
            .objects
            .classes
            .iter_mut()
            .find(|other| other.system == class.system && other.identity == class.identity)
        {
            class.generators = std::mem::take(&mut existing.generators);
            class.constructor = existing.constructor.take();
            class.help = class.help.or(existing.help.take());
            *existing = class;
        } else {
            self.objects.classes.push(class);
        }
    }

    fn class_representation(&mut self, binding: &str, value: &Robj) -> Option<InstalledClass> {
        if !matches!(value.kind, Rkind::S4) {
            return None;
        }
        let (system, field, kind) = if is_class(value, "refClassRepresentation") {
            (
                ObjectSystem::Reference,
                "fieldClasses",
                InstalledMemberKind::Field,
            )
        } else if is_class(value, "classRepresentation") {
            (ObjectSystem::S4, "slots", InstalledMemberKind::Slot)
        } else {
            return None;
        };
        let identity = object_name(value.attr("className")?)?;
        let name = identity.name.as_str();
        let mut members: Vec<_> = value
            .attr(field)
            .into_iter()
            .flat_map(named_list)
            .map(|(member, value)| {
                let (kind, declared_type) = if system == ObjectSystem::Reference
                    && scalar(value) == Some("activeBindingFunction")
                {
                    (InstalledMemberKind::ActiveBinding, None)
                } else {
                    (kind, object_name(value).map(ObjectType::Named))
                };
                self.member(name, member, kind, false, declared_type, None)
            })
            .collect();
        if system == ObjectSystem::Reference
            && let Some(methods) = value
                .attr("refMethods")
                .and_then(|env| self.environments.resolve(env))
        {
            for (member, method) in methods.bindings() {
                if method.attr("refClassName").and_then(scalar) != Some(name) {
                    continue;
                }
                let parameters = if is_class(method, "externalRefMethod") {
                    method
                        .attr("actual")
                        .and_then(formals)
                        .map(|mut parameters| {
                            // The wrapper supplies the receiver as the first argument.
                            if !parameters.is_empty() {
                                parameters.remove(0);
                            }
                            parameters
                        })
                } else if is_class(method, "refMethodDef") {
                    formals(method)
                } else {
                    continue;
                };
                members.push(self.member(
                    name,
                    member,
                    InstalledMemberKind::Method,
                    false,
                    None,
                    parameters,
                ));
            }
        }
        if system == ObjectSystem::Reference {
            members.extend(value.attr("slots").into_iter().flat_map(named_list).map(
                |(member, value)| {
                    self.member(
                        name,
                        member,
                        InstalledMemberKind::Slot,
                        false,
                        object_name(value).map(ObjectType::Named),
                        None,
                    )
                },
            ));
        }
        let parents = value
            .attr("contains")
            .into_iter()
            .flat_map(named_list)
            .filter_map(|(_, relation)| {
                match &relation.attr("distance")?.kind {
                    Rkind::Real(values) if values == &[1.0] => {}
                    Rkind::Int(values) if values == &[Some(1)] => {}
                    _ => return None,
                }
                object_name(relation.attr("superClass")?)
            })
            .collect();
        let help = (self.help)(&format!("{name}-class")).or_else(|| (self.help)(name));
        Some(InstalledClass {
            system,
            identity,
            origin: origin(binding, None),
            parents,
            parent_generator: None,
            parent_known: false,
            portable: None,
            cloneable: None,
            virtual_class: value.attr("virtual").and_then(logical),
            exported_class: false,
            generators: Vec::new(),
            constructor: None,
            constructor_returns_instance: true,
            members,
            help,
        })
    }

    fn s7_class(&self, binding: &str, value: &Robj) -> Option<InstalledClass> {
        let identity = s7_name(value)?;
        let members = value
            .attr("properties")
            .into_iter()
            .flat_map(named_list)
            .filter(|(_, property)| is_class(property, "S7_property"))
            .map(|(name, property)| {
                let declared_type = list_field(property, "class").and_then(s7_type);
                let mut member = self.member(
                    &identity.name,
                    name,
                    InstalledMemberKind::Property,
                    false,
                    declared_type,
                    None,
                );
                member.stored = list_field(property, "getter")
                    .is_some_and(|getter| matches!(getter.kind, Rkind::Null));
                member
            })
            .collect();
        Some(InstalledClass {
            system: ObjectSystem::S7,
            parents: value.attr("parent").and_then(s7_name).into_iter().collect(),
            parent_generator: None,
            parent_known: false,
            portable: None,
            cloneable: None,
            virtual_class: value.attr("abstract").and_then(logical),
            exported_class: false,
            generators: Vec::new(),
            constructor: formals(value),
            constructor_returns_instance: s7_constructor_returns_instance(value),
            members,
            help: (self.help)(binding).or_else(|| (self.help)(&identity.name)),
            identity,
            origin: origin(binding, None),
        })
    }

    fn member(
        &self,
        owner: &str,
        name: &str,
        kind: InstalledMemberKind,
        private: bool,
        declared_type: Option<ObjectType>,
        formals: Option<Vec<Formal>>,
    ) -> InstalledMember {
        InstalledMember {
            name: name.into(),
            kind,
            private,
            declared_type,
            formals,
            help: (self.help)(&format!("{owner}-{name}")),
            stored: matches!(kind, InstalledMemberKind::Field | InstalledMemberKind::Slot),
        }
    }

    fn s4_method(&mut self, binding: &str, entry: &str, method: &Robj) {
        if !is_class(method, "MethodDefinition") {
            return;
        }
        let Some(generic) = method.attr("generic").and_then(object_name) else {
            return;
        };
        let dispatch = method.attr("defined").and_then(s4_dispatch);
        let target = method.attr("target");
        let classes = target
            .and_then(strings)
            .map(|classes| classes.join(","))
            .unwrap_or_else(|| entry.replace('#', ","));
        let alias = format!("{},{classes}-method", generic.name);
        let target_dispatch = target.and_then(s4_dispatch);
        self.objects.methods.push(InstalledMethod {
            system: ObjectSystem::S4,
            generic,
            origin: origin(binding, Some(entry)),
            dispatch,
            target_dispatch,
            conditional: false,
            formals: formals(method),
            help: (self.help)(&alias),
        });
    }

    fn s7_methods(
        &mut self,
        binding: &str,
        table: &Environment,
        prefix: &str,
        depth: usize,
        remaining: &mut usize,
    ) {
        if depth >= 32 || *remaining == 0 {
            return;
        }
        for (key, value) in table.bindings() {
            if *remaining == 0 {
                break;
            }
            *remaining -= 1;
            let entry = if prefix.is_empty() {
                key.into()
            } else {
                format!("{prefix},{key}")
            };
            if let Some(nested) = self.environments.resolve(value) {
                self.s7_methods(binding, &nested, &entry, depth + 1, remaining);
            } else {
                self.s7_method(binding, &entry, value);
            }
        }
    }

    fn s7_method(&mut self, binding: &str, entry: &str, method: &Robj) {
        if !is_class(method, "S7_method") {
            return;
        }
        let Some(generic) = method.attr("generic") else {
            return;
        };
        let Some(mut identity) = generic.attr("name").and_then(object_name) else {
            return;
        };
        identity.package = identity.package.or_else(|| closure_namespace(generic));
        let dispatch = s7_dispatch(method.attr("signature"), generic.attr("dispatch_args"));
        self.objects.methods.push(InstalledMethod {
            system: ObjectSystem::S7,
            generic: identity,
            origin: origin(binding, Some(entry)),
            dispatch,
            target_dispatch: None,
            conditional: false,
            formals: formals(method),
            help: (self.help)(&format!("{binding},{entry}-method")),
        });
    }

    fn s7_external_method(&mut self, binding: &str, index: usize, record: &Robj) {
        let Some(generic) = list_field(record, "generic") else {
            return;
        };
        if !is_class(generic, "S7_external_generic") {
            return;
        }
        let (Some(name), Some(package), Some(method)) = (
            list_field(generic, "name").and_then(scalar),
            list_field(generic, "package").and_then(scalar),
            list_field(record, "method"),
        ) else {
            return;
        };
        let args =
            list_field(generic, "dispatch_args").filter(|args| scalar(args) != Some("__S3__"));
        let dispatch = s7_dispatch(list_field(record, "signature"), args);
        self.objects.methods.push(InstalledMethod {
            system: ObjectSystem::S7,
            generic: ObjectName {
                name: name.into(),
                package: Some(package.into()),
            },
            origin: origin(binding, Some(&(index + 1).to_string())),
            dispatch,
            target_dispatch: None,
            // The package still needs to replay this registration.
            conditional: true,
            formals: formals(method),
            help: None,
        });
    }
}

fn s7_constructor_returns_instance(value: &Robj) -> bool {
    let Rkind::Closure { body, .. } = &value.kind else {
        return false;
    };
    let body = match &body.kind {
        Rkind::Bytecode(source) => source,
        _ => body,
    };
    let Rkind::Pairlist(items) = &body.kind else {
        return false;
    };
    let call = if matches!(items.first().map(|item| &item.value.kind),
        Some(Rkind::Symbol(name)) if name == "{")
    {
        let Some(tail) = items.last() else {
            return false;
        };
        let Rkind::Pairlist(call) = &tail.value.kind else {
            return false;
        };
        call
    } else {
        items
    };
    call.first()
        .and_then(|callee| binding_reference(&callee.value))
        .is_some_and(|callee| {
            callee.package.as_deref() == Some("S7") && callee.name == "new_object"
        })
}

fn logical(value: &Robj) -> Option<bool> {
    match &value.kind {
        Rkind::Logical(values) => match values.as_slice() {
            [value] => *value,
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rindex::rds::PairlistItem;

    fn bare(kind: Rkind) -> Robj {
        Robj { kind, attr: vec![] }
    }

    fn strings(values: &[&str]) -> Robj {
        bare(Rkind::Str(
            values.iter().map(|value| Some((*value).into())).collect(),
        ))
    }

    fn call(values: Vec<Robj>) -> Robj {
        bare(Rkind::Pairlist(
            values
                .into_iter()
                .map(|value| PairlistItem { tag: None, value })
                .collect(),
        ))
    }

    fn attributed(kind: Rkind, attrs: &[(&str, Robj)]) -> Robj {
        Robj {
            kind,
            attr: attrs
                .iter()
                .map(|(name, value)| ((*name).into(), value.clone()))
                .collect(),
        }
    }

    fn named(values: &[(&str, Robj)], class: &str) -> Robj {
        let names: Vec<_> = values.iter().map(|(name, _)| *name).collect();
        attributed(
            Rkind::List(values.iter().map(|(_, value)| value.clone()).collect()),
            &[("names", strings(&names)), ("class", strings(&[class]))],
        )
    }

    fn tagged(values: &[(&str, Robj)]) -> Rkind {
        Rkind::Pairlist(
            values
                .iter()
                .map(|(name, value)| PairlistItem {
                    tag: (!name.is_empty()).then(|| (*name).into()),
                    value: value.clone(),
                })
                .collect(),
        )
    }

    fn environment(values: &[(&str, Robj)]) -> Rkind {
        Rkind::Environment(std::sync::Arc::new(crate::rindex::rds::Environment {
            parent: bare(Rkind::Env),
            frame: bare(tagged(values)),
            hash_table: bare(Rkind::Null),
            attr: vec![],
        }))
    }

    fn closure(parameters: &[(&str, Robj)], body: Robj) -> Rkind {
        Rkind::Closure {
            formals: Box::new(bare(tagged(parameters))),
            body: Box::new(body),
            environment: None,
        }
    }

    #[test]
    fn s3_use_method_matches_named_arguments_first() {
        let name = strings(&["foo"]);
        let symbol = bare(Rkind::Symbol("x".into()));
        let literal = strings(&["wrong"]);
        for (arguments, expected) in [
            (vec![("object", &symbol), ("generic", &name)], Some("x")),
            (vec![("object", &literal), ("generic", &name)], None),
            (vec![("object", &symbol), ("", &name)], Some("x")),
            (vec![("", &symbol), ("generic", &name)], Some("x")),
            (vec![("o", &symbol), ("g", &name)], Some("x")),
            (vec![("", &name), ("", &symbol)], Some("x")),
        ] {
            let mut call = vec![("", bare(Rkind::Symbol("UseMethod".into())))];
            call.extend(
                arguments
                    .into_iter()
                    .map(|(tag, value)| (tag, value.clone())),
            );
            let value = bare(closure(&[], bare(tagged(&call))));
            let mut harvester = ObjectHarvester::new(None, "example", false, &|_| None);
            harvester.observe("foo", &value);
            let [generic] = harvester.objects.generics.as_slice() else {
                panic!("expected one generic");
            };
            assert_eq!(generic.identity.name, "foo");
            assert_eq!(
                generic.dispatch_arguments,
                expected.map(|name| vec![name.into()])
            );
        }
    }

    #[test]
    fn reference_class_harvests_explicit_receiver_methods() {
        let actual = bare(closure(
            &[
                (".self", bare(Rkind::Missing)),
                ("value", bare(Rkind::Int(vec![Some(42)]))),
            ],
            bare(Rkind::Null),
        ));
        let method = attributed(
            closure(&[("...", bare(Rkind::Missing))], bare(Rkind::Null)),
            &[
                ("class", strings(&["externalRefMethod"])),
                ("refClassName", strings(&["Counter"])),
                ("actual", actual),
            ],
        );
        let class = attributed(
            Rkind::S4,
            &[
                ("class", strings(&["refClassRepresentation"])),
                ("className", strings(&["Counter"])),
                ("refMethods", bare(environment(&[("set", method)]))),
            ],
        );
        let mut harvester = ObjectHarvester::new(None, "example", false, &|_| None);
        harvester.observe(".__C__Counter", &class);
        let members = &harvester.objects.classes[0].members;
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].kind, InstalledMemberKind::Method);
        assert_eq!(
            members[0].formals.as_deref(),
            Some(
                [Formal {
                    name: "value".into(),
                    default: Some("42L".into())
                }]
                .as_slice()
            )
        );
    }

    #[test]
    fn reference_class_distinguishes_active_bindings_from_fields() {
        let class = attributed(
            Rkind::S4,
            &[
                ("class", strings(&["refClassRepresentation"])),
                ("className", strings(&["Counter"])),
                (
                    "fieldClasses",
                    named(
                        &[
                            ("computed", strings(&["activeBindingFunction"])),
                            ("value", strings(&["integer"])),
                        ],
                        "list",
                    ),
                ),
            ],
        );
        let mut harvester = ObjectHarvester::new(None, "example", false, &|_| None);
        harvester.observe(".__C__Counter", &class);
        let members = &harvester.objects.classes[0].members;
        assert_eq!(members[0].kind, InstalledMemberKind::ActiveBinding);
        assert_eq!(members[0].declared_type, None);
        assert_eq!(members[1].kind, InstalledMemberKind::Field);
        assert_eq!(
            members[1].declared_type,
            Some(ObjectType::Named(ObjectName {
                name: "integer".into(),
                package: None
            }))
        );
    }

    #[test]
    fn s7_class_keeps_the_direct_s3_parent() {
        let parent = named(&[("class", strings(&["Child", "Parent"]))], "S7_S3_class");
        let class = attributed(
            closure(&[], bare(Rkind::Null)),
            &[
                ("class", strings(&["S7_class"])),
                ("name", strings(&["Leaf"])),
                ("parent", parent),
            ],
        );
        let mut harvester = ObjectHarvester::new(None, "example", false, &|_| None);
        harvester.observe("Leaf", &class);
        assert_eq!(
            harvester.objects.classes[0].parents,
            vec![ObjectName {
                name: "Child".into(),
                package: None
            }]
        );
    }

    #[test]
    fn s7_nested_methods_keep_paths_and_s4_dispatch_classes() {
        let generic_attrs = [
            ("class", strings(&["S7_generic"])),
            ("name", strings(&["combine"])),
            ("dispatch_args", strings(&["x", "y"])),
        ];
        let generic = attributed(closure(&[], bare(Rkind::Null)), &generic_attrs);
        let s4_class = attributed(
            Rkind::S4,
            &[
                ("class", strings(&["classRepresentation"])),
                ("className", strings(&["Record"])),
                ("package", strings(&["data"])),
            ],
        );
        let any = attributed(Rkind::List(vec![]), &[("class", strings(&["S7_any"]))]);
        let method = attributed(
            closure(&[], bare(Rkind::Null)),
            &[
                ("class", strings(&["S7_method"])),
                ("generic", generic.clone()),
                ("signature", bare(Rkind::List(vec![s4_class, any]))),
            ],
        );
        let table = bare(environment(&[(
            "S4/data::Record",
            bare(environment(&[("ANY", method)])),
        )]));
        let mut generic_with_methods = generic;
        generic_with_methods.attr.push(("methods".into(), table));
        let mut harvester = ObjectHarvester::new(None, "example", false, &|_| None);
        harvester.observe("combine", &generic_with_methods);
        let [method] = harvester.objects.methods.as_slice() else {
            panic!("expected one method");
        };
        assert_eq!(method.origin.entry.as_deref(), Some("S4/data::Record,ANY"));
        assert_eq!(
            method.dispatch,
            Some(vec![
                DispatchParameter {
                    argument: Some("x".into()),
                    class: Some(ObjectType::Named(ObjectName {
                        name: "Record".into(),
                        package: Some("data".into()),
                    })),
                },
                DispatchParameter {
                    argument: Some("y".into()),
                    class: Some(ObjectType::Any)
                },
            ])
        );
    }

    #[test]
    fn s7_external_s3_dispatch_has_no_argument_marker() {
        let generic = named(
            &[
                ("name", strings(&["format"])),
                ("package", strings(&["base"])),
                ("dispatch_args", strings(&["__S3__"])),
            ],
            "S7_external_generic",
        );
        let date = named(&[("class", strings(&["Date"]))], "S7_S3_class");
        let record = named(
            &[
                ("generic", generic),
                ("signature", bare(Rkind::List(vec![date]))),
                ("method", bare(closure(&[], bare(Rkind::Null)))),
            ],
            "list",
        );
        let table = bare(Rkind::Environment(std::sync::Arc::new(
            crate::rindex::rds::Environment {
                parent: bare(Rkind::Env),
                frame: bare(Rkind::Null),
                hash_table: bare(Rkind::Null),
                attr: vec![("S7methods".into(), bare(Rkind::List(vec![record])))],
            },
        )));
        let mut harvester = ObjectHarvester::new(None, "example", false, &|_| None);
        harvester.observe(".__S3MethodsTable__.", &table);
        assert_eq!(
            harvester.objects.methods[0].dispatch,
            Some(vec![DispatchParameter {
                argument: None,
                class: Some(ObjectType::S3Chain(vec!["Date".into()])),
            }])
        );
    }

    #[test]
    fn s4_help_uses_target_signature_order() {
        let method = Robj {
            kind: Rkind::Opaque,
            attr: vec![
                ("class".into(), strings(&["MethodDefinition"])),
                ("generic".into(), strings(&["foo"])),
                ("defined".into(), strings(&["A", "ANY"])),
                ("target".into(), strings(&["A", "numeric"])),
            ],
        };
        let mut harvester = ObjectHarvester::new(None, "example", false, &|alias| {
            (alias == "foo,A,numeric-method").then(|| HelpAssociation {
                alias: alias.into(),
                topic: None,
            })
        });
        harvester.s4_method(".__T__foo:example", "A#numeric", &method);
        let method = &harvester.objects.methods[0];
        assert_eq!(method.origin.entry.as_deref(), Some("A#numeric"));
        assert_eq!(method.help.as_ref().unwrap().alias, "foo,A,numeric-method");
    }

    #[test]
    fn s3_generic_rejects_shadowed_bare_use_method() {
        let symbol = |name: &str| bare(Rkind::Symbol(name.into()));
        let parameter = || PairlistItem {
            tag: Some("UseMethod".into()),
            value: bare(Rkind::Missing),
        };
        let closure = |callee, formals, environment| {
            bare(Rkind::Closure {
                formals: Box::new(formals),
                body: Box::new(call(vec![callee, strings(&["foo"])])),
                environment,
            })
        };
        let mut harvester = ObjectHarvester::new(None, "example", false, &|_| None);
        harvester.observe(
            "foo",
            &closure(
                symbol("UseMethod"),
                bare(Rkind::Pairlist(vec![parameter()])),
                None,
            ),
        );

        let captured = bare(Rkind::Environment(std::sync::Arc::new(
            crate::rindex::rds::Environment {
                frame: bare(Rkind::Pairlist(vec![PairlistItem {
                    value: bare(Rkind::Builtin),
                    ..parameter()
                }])),
                hash_table: bare(Rkind::Null),
                parent: bare(Rkind::Env),
                attr: vec![],
            },
        )));
        harvester.observe(
            "foo",
            &closure(
                symbol("UseMethod"),
                bare(Rkind::Null),
                Some(Box::new(captured)),
            ),
        );
        assert!(harvester.objects.generics.is_empty());

        let qualified = call(vec![symbol("::"), symbol("base"), symbol("UseMethod")]);
        harvester.observe(
            "foo",
            &closure(qualified, bare(Rkind::Pairlist(vec![parameter()])), None),
        );
        harvester.observe(
            "foo",
            &closure(symbol("UseMethod"), bare(Rkind::Null), None),
        );
        assert_eq!(harvester.objects.generics.len(), 2);

        let mut harvester = ObjectHarvester::new(None, "example", true, &|_| None);
        let namespace = bare(Rkind::Namespace(vec![Some("example".into())]));
        harvester.observe(
            "foo",
            &closure(
                symbol("UseMethod"),
                bare(Rkind::Null),
                Some(Box::new(namespace)),
            ),
        );
        assert!(harvester.objects.generics.is_empty());
    }
}
