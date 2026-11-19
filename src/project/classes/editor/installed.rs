//! Installed objects have package origins and no source locations.

use super::*;
use crate::rindex::provider::{package_exports_name, package_indexed, resolve_origin};
use crate::rindex::schema::{
    InstalledClass, InstalledGeneric, InstalledMemberKind, ObjectName, ObjectSystem, ObjectType,
};
use crate::semantic::{LoadedPackage, PackageOrigin};

pub(in crate::project::classes) struct InstalledObjects<'a> {
    pub(super) indexed: &'a IndexedProvider,
    pub(super) remote: &'a RemoteExports,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::project::classes) struct InstalledReceiver<'a> {
    pub(super) package: &'a PackageIndex,
    pub(super) class: &'a InstalledClass,
    pub(super) access: ClassAccess,
    pub(in crate::project::classes) exact: bool,
}

impl<'a> InstalledReceiver<'a> {
    pub(in crate::project::classes) fn construct(mut self, member: bool) -> Option<Self> {
        if self.access != ClassAccess::Generator || self.class.virtual_class == Some(true) {
            return None;
        }
        match (self.class.system, member) {
            (ObjectSystem::R6, true) | (ObjectSystem::Reference, _) => {}
            (ObjectSystem::S7, false) => self.exact &= self.class.constructor_returns_instance,
            _ => return None,
        }
        self.access = ClassAccess::Public;
        Some(self)
    }
}

impl<'a> InstalledObjects<'a> {
    fn package_binding(
        &self,
        index: &ClassIndex,
        path: &Path,
        reference: &FunctionRef,
        internal: bool,
        at: &SyntaxElement,
    ) -> Option<(&'a PackageIndex, String)> {
        let (package, name) = match reference {
            FunctionRef::Qualified { package, name } => (package.clone(), name),
            FunctionRef::Bare {
                name,
                attached,
                attachment_uncertain: false,
            } => {
                if index.installed_binding_masked(path, at) {
                    return None;
                }
                let root = index.file(path)?.package_root.as_ref();
                let namespace = root.and_then(|root| index.s3_origins.namespaces.get(root));
                if namespace.is_some_and(|namespace| namespace.unknown) {
                    return None;
                }
                let named: Vec<_> = namespace
                    .and_then(|namespace| namespace.named.get(name))
                    .into_iter()
                    .flatten()
                    .collect();
                if let [package] = named.as_slice() {
                    (package.to_string(), name)
                } else if !named.is_empty() {
                    return None;
                } else {
                    let wildcard: Vec<_> = namespace
                        .into_iter()
                        .flat_map(|namespace| &namespace.wildcard)
                        .filter(|(_, excluded)| !excluded.contains(name))
                        .map(|(package, _)| package)
                        .collect();
                    if wildcard
                        .iter()
                        .any(|package| !package_indexed(self.indexed, self.remote, package))
                    {
                        return None;
                    }
                    let imports: Vec<_> = wildcard
                        .into_iter()
                        .filter(|package| {
                            package_exports_name(self.indexed, self.remote, package, name)
                        })
                        .collect();
                    if let [package] = imports.as_slice() {
                        (package.to_string(), name)
                    } else if !imports.is_empty() {
                        return None;
                    } else {
                        let attached: Vec<_> = root
                            .and_then(|root| index.s3_origins.attached.get(root))
                            .into_iter()
                            .flatten()
                            .chain(attached)
                            .map(|package| LoadedPackage {
                                name: package.as_str().into(),
                                range: TextRange::default(),
                            })
                            .collect();
                        if attached
                            .iter()
                            .any(|pkg| !package_indexed(self.indexed, self.remote, &pkg.name))
                        {
                            return None;
                        }
                        let package =
                            match resolve_origin(self.indexed, self.remote, name, &attached) {
                                PackageOrigin::Resolved(owner) => owner,
                                PackageOrigin::Ambiguous(owners) => owners.into_iter().last()?,
                                PackageOrigin::Unknown => return None,
                            };
                        (package.to_string(), name)
                    }
                }
            }
            _ => return None,
        };
        if index.s4_workspace_package(&package) {
            return None;
        }
        let package = self.indexed.package(&package)?;
        (internal || exported(package, name)).then(|| (package, name.clone()))
    }

    pub(in crate::project::classes) fn direct_receiver(
        &self,
        index: &ClassIndex,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: &SyntaxElement,
    ) -> Option<InstalledReceiver<'a>> {
        if let Some(call) = value.as_node().cloned().and_then(CallExpr::cast) {
            let reference = call_function_ref(model, root, &call)?;
            let name = match &reference {
                FunctionRef::Qualified { package, name } if package == "methods" => name.as_str(),
                _ => {
                    let origin = index.callee_identity(path, &reference)?;
                    if origin.package != "methods" {
                        return None;
                    }
                    &origin.name
                }
            };
            let args = match name {
                "new" => matched_args(&call, &["Class", "..."])?,
                "getRefClass" => matched_args(&call, &["Class", "where"])?,
                _ => return None,
            };
            let qualifier = call.callee_token()?;
            let mut receiver = self.formal_class(
                index,
                path,
                root,
                model,
                &argument(&args, "Class")?,
                Some(&qualifier),
            )?;
            if name == "new" {
                if receiver.class.virtual_class == Some(true) {
                    return None;
                }
                receiver.access = ClassAccess::Public;
                receiver.exact = receiver.class.virtual_class == Some(false)
                    || receiver.class.system == ObjectSystem::Reference;
            } else if receiver.class.system != ObjectSystem::Reference || args.contains_key("where")
            {
                return None;
            }
            return Some(receiver);
        }
        let internal = value
            .as_node()
            .cloned()
            .and_then(BinaryExpr::cast)
            .and_then(|binary| binary.namespace_access())
            .is_some_and(|namespace| namespace.internal);
        let reference = function_ref(model, root, value, &mut BTreeSet::new())?;
        let (package, name) = self.package_binding(index, path, &reference, internal, value)?;
        let class = unique(package.objects.classes.iter().filter(|class| {
            match class.system {
                ObjectSystem::R6 | ObjectSystem::S7 => class.origin.binding == name,
                ObjectSystem::Reference => class
                    .generators
                    .iter()
                    .any(|generator| generator.binding == name),
                _ => false,
            }
        }))?;
        Some(InstalledReceiver {
            package,
            class,
            access: ClassAccess::Generator,
            exact: true,
        })
    }

    pub(super) fn formal_class(
        &self,
        index: &ClassIndex,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: &SyntaxElement,
        callee: Option<&SyntaxToken>,
    ) -> Option<InstalledReceiver<'a>> {
        let token = value
            .as_token()
            .filter(|token| token.kind() == SyntaxKind::STRING)?;
        let name = crate::semantic::decode_string_literal(token.text())?;
        let caller = index.file(path)?.package_root.as_ref();
        let mut packages = BTreeSet::new();
        if let Some(root) = caller {
            if let Some(namespace) = index.s4_namespace(root) {
                if !namespace.complete {
                    return None;
                }
                packages.extend(
                    namespace
                        .imported_classes
                        .iter()
                        .filter(|(_, classes)| classes.contains(&name))
                        .map(|(pkg, _)| pkg.clone()),
                );
            }
            if let Some(namespace) = index.s3_origins.namespaces.get(root) {
                packages.extend(namespace.wildcard.iter().map(|(pkg, _)| pkg.clone()));
            }
            packages.extend(
                index
                    .s3_origins
                    .attached
                    .get(root)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
        }
        let reference = free_function_ref(model, root, callee.unwrap_or(token), &name)?;
        if let FunctionRef::Bare {
            attached,
            attachment_uncertain: false,
            ..
        } = reference
        {
            packages.extend(attached);
        } else {
            return None;
        }
        // A competing visible workspace definition cannot be bypassed by a cache hit.
        if !index
            .resolve_reference(
                path,
                &ClassOwner::default(),
                &ParentRef::Formal(name.clone()),
                true,
            )
            .is_empty()
        {
            return None;
        }
        let mut candidates = Vec::new();
        for package in packages {
            if index.s4_workspace_package(&package) {
                continue;
            }
            let Some(package) = self.indexed.package(&package) else {
                if matches!(package.as_str(), "base" | "methods") {
                    continue;
                }
                return None;
            };
            candidates.extend(
                package
                    .objects
                    .classes
                    .iter()
                    .filter(|class| {
                        class.exported_class
                            && class.identity.name == name
                            && matches!(class.system, ObjectSystem::S4 | ObjectSystem::Reference)
                    })
                    .map(|class| InstalledReceiver {
                        package,
                        class,
                        access: ClassAccess::Generator,
                        exact: true,
                    }),
            );
        }
        unique(candidates.into_iter())
    }

    pub(super) fn generic(
        &self,
        index: &ClassIndex,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: &SyntaxElement,
    ) -> Option<(&'a PackageIndex, &'a InstalledGeneric)> {
        let internal = value
            .as_node()
            .cloned()
            .and_then(BinaryExpr::cast)
            .and_then(|binary| binary.namespace_access())
            .is_some_and(|namespace| namespace.internal);
        let reference = function_ref(model, root, value, &mut BTreeSet::new())?;
        let (package, name) = self.package_binding(index, path, &reference, internal, value)?;
        let generic = unique(
            package
                .objects
                .generics
                .iter()
                .filter(|generic| generic.origin.binding == name),
        )?;
        Some((package, generic))
    }

    pub(super) fn members(&self, receiver: InstalledReceiver<'a>, at: bool) -> MemberSurface {
        if receiver.access == ClassAccess::Generator {
            if at {
                return MemberSurface::default();
            }
            let mut surface = MemberSurface {
                complete: true,
                ..MemberSurface::default()
            };
            match receiver.class.system {
                ObjectSystem::R6 => {
                    for (name, members) in r6::generator_members().entries {
                        for entry in members {
                            let mut member = EditorMember::new(entry.kind);
                            member.signature = entry.signature;
                            surface.insert(name.clone(), member);
                        }
                    }
                }
                ObjectSystem::Reference => {
                    append_reference_members(&mut surface, reference::generator_members())
                }
                _ => {}
            }
            if let Some(new) = surface.members.get_mut("new") {
                new.signature = self.constructor(receiver).or_else(|| new.signature.take());
                new.help = receiver
                    .class
                    .help
                    .as_ref()
                    .map(|help| ObjectHelp::new(receiver.package, help));
            }
            for member in surface.members.values_mut() {
                member.origin = Some(receiver.package.package.clone());
            }
            return surface;
        }
        let mut lineage = Vec::new();
        let complete = self.lineage(receiver, &mut BTreeSet::new(), &mut lineage);
        let mut surface = MemberSurface {
            complete,
            ..MemberSurface::default()
        };
        if receiver.class.system == ObjectSystem::Reference && !at {
            append_reference_members(&mut surface, reference::object_members());
        }
        if receiver.class.system == ObjectSystem::R6
            && !at
            && receiver.class.cloneable == Some(true)
        {
            let parent_agrees = lineage.len() == 1
                || lineage
                    .iter()
                    .rev()
                    .nth(1)
                    .is_some_and(|parent| parent.class.cloneable == Some(true));
            if complete && parent_agrees {
                let mut clone = EditorMember::new(MemberKind::Method);
                clone.signature = Some(CallableSignature {
                    parameters: vec![CallableParameter {
                        name: "deep".into(),
                        default: Some("FALSE".into()),
                    }],
                });
                surface.insert("clone".into(), clone);
            }
        }
        // R6 applies method and field overrides in separate layers. A field
        // of the same name wins even when declared in a different ancestor.
        let mut r6_fields = BTreeMap::new();
        for owner in lineage {
            let mut own = MemberSurface::default();
            for entry in &owner.class.members {
                if (entry.kind == InstalledMemberKind::Slot
                    || entry.kind == InstalledMemberKind::Property)
                    != at
                    || (entry.private && receiver.access == ClassAccess::Public)
                    || (!entry.private && receiver.access == ClassAccess::Private)
                {
                    continue;
                }
                let kind = match entry.kind {
                    InstalledMemberKind::Field => MemberKind::Field,
                    InstalledMemberKind::Slot => MemberKind::Slot,
                    InstalledMemberKind::Property => MemberKind::Property,
                    InstalledMemberKind::Method => MemberKind::Method,
                    InstalledMemberKind::ActiveBinding => MemberKind::ActiveBinding,
                };
                let mut member = EditorMember::new(kind);
                member.origin = Some(owner.package.package.clone());
                if complete || receiver.class.system != ObjectSystem::R6 {
                    if kind == MemberKind::Method {
                        member.signature = entry.formals.as_deref().map(from_formals);
                    }
                    member.declared_type = entry.declared_type.as_ref().map(object_type_label);
                    member.help = entry
                        .help
                        .as_ref()
                        .map(|help| ObjectHelp::new(owner.package, help));
                }
                if receiver.class.system == ObjectSystem::R6
                    && entry.kind == InstalledMemberKind::Field
                {
                    r6_fields.insert(entry.name.to_string(), member);
                } else {
                    own.insert(entry.name.to_string(), member);
                }
            }
            for (name, member) in own.members {
                if member.kind == MemberKind::ActiveBinding && surface.members.contains_key(&name) {
                    surface.insert(name, member);
                    surface.complete = false;
                } else {
                    surface.members.insert(name, member);
                }
            }
        }
        for (name, field) in r6_fields {
            if surface
                .members
                .get(&name)
                .is_some_and(|member| member.kind == MemberKind::ActiveBinding)
            {
                surface.insert(name, field);
                surface.complete = false;
            } else {
                surface.members.insert(name, field);
            }
        }
        surface
    }

    pub(super) fn constructor(&self, receiver: InstalledReceiver<'a>) -> Option<CallableSignature> {
        if receiver.class.virtual_class == Some(true) {
            return None;
        }
        match receiver.class.system {
            ObjectSystem::R6 | ObjectSystem::Reference => {
                let instance = InstalledReceiver {
                    access: ClassAccess::Public,
                    ..receiver
                };
                let surface = self.members(instance, false);
                if let Some(member) = surface.members.get("initialize")
                    && let Some(signature) = &member.signature
                {
                    return Some(signature.clone());
                }
                if receiver.class.system == ObjectSystem::R6 && surface.complete {
                    return Some(simple_signature(&[]));
                }
                receiver.class.constructor.as_deref().map(from_formals)
            }
            _ => receiver.class.constructor.as_deref().map(from_formals),
        }
    }

    pub(in crate::project::classes) fn member_type(
        &self,
        receiver: InstalledReceiver<'a>,
        at: bool,
        name: &str,
    ) -> Option<InstalledReceiver<'a>> {
        let mut lineage = Vec::new();
        self.lineage(receiver, &mut BTreeSet::new(), &mut lineage);
        for owner in lineage.into_iter().rev() {
            let Some(member) = unique(owner.class.members.iter().filter(|m| m.name == name)) else {
                if owner.class.members.iter().any(|m| m.name == name) {
                    return None;
                }
                continue;
            };
            if member.private
                || !matches!(
                    (member.kind, at),
                    (
                        InstalledMemberKind::Slot | InstalledMemberKind::Property,
                        true
                    ) | (InstalledMemberKind::Field, false)
                )
                || !member.stored
            {
                return None;
            }
            let ObjectType::Named(class) = member.declared_type.as_ref()? else {
                return None;
            };
            let mut result = self.parent(owner, class, false)?;
            result.access = ClassAccess::Public;
            result.exact = false;
            return Some(result);
        }
        None
    }

    fn lineage(
        &self,
        receiver: InstalledReceiver<'a>,
        visiting: &mut BTreeSet<(&'a str, &'a str)>,
        out: &mut Vec<InstalledReceiver<'a>>,
    ) -> bool {
        let key = (
            receiver.package.package.as_str(),
            receiver.class.origin.binding.as_str(),
        );
        if out.iter().any(|prior| {
            prior.package.package == receiver.package.package
                && prior.class.origin == receiver.class.origin
        }) {
            return true;
        }
        if visiting.len() >= 64 || out.len() >= 64 || !visiting.insert(key) {
            return false;
        }
        let class = receiver.class;
        let mut complete = class.system != ObjectSystem::R6 || class.parent_known;
        for parent in class
            .parents
            .iter()
            .rev()
            .chain(class.parent_generator.iter())
        {
            if matches!(parent.name.as_str(), "envRefClass" | "S7_object")
                && matches!(parent.package.as_deref(), Some("methods" | "S7"))
            {
                continue;
            }
            let binding = class.system == ObjectSystem::R6;
            if let Some(parent) = self.parent(receiver, parent, binding) {
                if binding {
                    complete &= class.portable.is_some() && class.portable == parent.class.portable;
                }
                complete &= self.lineage(parent, visiting, out);
            } else {
                complete = false;
            }
        }
        visiting.remove(&key);
        out.push(receiver);
        complete
    }

    fn parent(
        &self,
        owner: InstalledReceiver<'a>,
        name: &ObjectName,
        binding: bool,
    ) -> Option<InstalledReceiver<'a>> {
        let package = match name.package.as_deref() {
            Some(package) if package != owner.package.package => self.indexed.package(package)?,
            _ => owner.package,
        };
        let class = unique(package.objects.classes.iter().filter(|class| {
            if binding {
                class.system == ObjectSystem::R6 && class.origin.binding == name.name
            } else {
                class.identity.name == name.name
                    && name
                        .package
                        .as_ref()
                        .is_none_or(|pkg| class.identity.package.as_ref() == Some(pkg))
            }
        }))?;
        Some(InstalledReceiver {
            package,
            class,
            ..owner
        })
    }
}

fn append_reference_members(surface: &mut MemberSurface, entries: ReferenceMembers) {
    for (name, members) in entries.entries {
        for entry in members {
            let mut member = EditorMember::new(entry.kind);
            member.signature = entry.signature;
            surface.insert(name.clone(), member);
        }
    }
}

fn unique<T>(mut values: impl Iterator<Item = T>) -> Option<T> {
    let first = values.next()?;
    values.next().is_none().then_some(first)
}

fn exported(package: &PackageIndex, name: &str) -> bool {
    package
        .symbols
        .iter()
        .any(|entry| entry.exported && entry.name == name)
}

fn object_type_label(class: &ObjectType) -> String {
    match class {
        ObjectType::Named(name) => name.name.to_string(),
        ObjectType::S3Chain(names) => names.join(", "),
        ObjectType::Union(classes) => classes
            .iter()
            .map(object_type_label)
            .collect::<Vec<_>>()
            .join(" | "),
        ObjectType::Any => "ANY".into(),
        ObjectType::Missing => "missing".into(),
    }
}
