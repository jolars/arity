//! Request-local callable and member surfaces shared by editor reads.

use super::*;
use crate::rindex::provider::IndexedProvider;
use crate::rindex::remote::RemoteExports;
use crate::rindex::schema::{Formal, HelpAssociation, HelpDoc, PackageIndex};
use smol_str::SmolStr;

mod installed;
pub(super) use installed::{InstalledObjects, InstalledReceiver};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ObjectHelp {
    package: SmolStr,
    version: SmolStr,
    association: HelpAssociation,
}

impl ObjectHelp {
    fn new(package: &PackageIndex, association: &HelpAssociation) -> Self {
        Self {
            package: package.package.clone(),
            version: package.version.clone(),
            association: association.clone(),
        }
    }

    pub(crate) fn resolve<'a>(&self, indexed: &'a IndexedProvider) -> Option<&'a HelpDoc> {
        let package = indexed.package(&self.package)?;
        (package.version == self.version)
            .then(|| package.objects.render_help(&self.association))
            .flatten()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum EditorReceiver<'a> {
    Source(ClassReceiver),
    Installed(InstalledReceiver<'a>),
}

#[derive(Clone)]
pub(crate) struct EditorMember {
    pub kind: MemberKind,
    pub signature: Option<CallableSignature>,
    pub declared_type: Option<String>,
    pub help: Option<ObjectHelp>,
    pub origin: Option<SmolStr>,
}

impl EditorMember {
    fn new(kind: MemberKind) -> Self {
        Self {
            kind,
            signature: None,
            declared_type: None,
            help: None,
            origin: None,
        }
    }
}

#[derive(Default)]
pub(crate) struct MemberSurface {
    pub members: BTreeMap<String, EditorMember>,
    pub complete: bool,
}

impl MemberSurface {
    fn insert(&mut self, name: String, member: EditorMember) {
        use std::collections::btree_map::Entry;
        match self.members.entry(name) {
            Entry::Vacant(entry) => {
                entry.insert(member);
            }
            Entry::Occupied(mut entry) => {
                let previous = entry.get_mut();
                if previous.kind != member.kind {
                    previous.kind = MemberKind::Unknown;
                }
                if previous.origin != member.origin {
                    previous.origin = None;
                }
                previous.signature = None;
                previous.declared_type = None;
                previous.help = None;
            }
        }
    }
}

pub(crate) struct EditorCallable {
    pub name: String,
    pub signature: CallableSignature,
    pub help: Option<ObjectHelp>,
    pub notes: Vec<String>,
}

pub(crate) struct EditorRead<'a> {
    index: &'a ClassIndex,
    root: &'a SyntaxNode,
    model: &'a SemanticModel,
    path: &'a Path,
    installed: InstalledObjects<'a>,
    extracted: Extracted,
}

impl ClassIndex {
    /// Resolve the callable surface of a source class, member, or generic.
    pub fn class_call_signature(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        call: &CallExpr,
    ) -> Option<(String, CallableSignature)> {
        let indexed = IndexedProvider::empty();
        let remote = RemoteExports::default();
        let callable =
            EditorRead::new(self, root, model, path, &indexed, &remote).callable(call)?;
        Some((callable.name, callable.signature))
    }
}

impl<'a> EditorRead<'a> {
    pub(crate) fn new(
        index: &'a ClassIndex,
        root: &'a SyntaxNode,
        model: &'a SemanticModel,
        path: &'a Path,
        indexed: &'a IndexedProvider,
        remote: &'a RemoteExports,
    ) -> Self {
        Self {
            index,
            root,
            model,
            path,
            installed: InstalledObjects { indexed, remote },
            extracted: extract(model, root, false),
        }
    }

    fn receivers(&self, value: &SyntaxElement) -> ReceiverEvidence<EditorReceiver<'a>> {
        self.index.editor_receivers(
            self.path,
            self.root,
            self.model,
            value,
            &self.extracted,
            Some(&self.installed),
        )
    }

    pub(crate) fn class_hover(
        &self,
        value: &SyntaxElement,
    ) -> Option<(String, Option<CallableSignature>, Option<ObjectHelp>)> {
        let receiver = match self.receivers(value).unique()? {
            EditorReceiver::Installed(receiver) => receiver,
            EditorReceiver::Source(receiver) => {
                let class = &self.index.declarations.get(&receiver.class)?.declaration;
                return Some((
                    format!("{} `{}`", class.system().label(), class.display_name()),
                    None,
                    None,
                ));
            }
        };
        let role = match receiver.class.system {
            crate::rindex::schema::ObjectSystem::S3 => "S3 class",
            crate::rindex::schema::ObjectSystem::S4 => "S4 class",
            crate::rindex::schema::ObjectSystem::Reference => "reference class",
            crate::rindex::schema::ObjectSystem::R6 => "R6 class",
            crate::rindex::schema::ObjectSystem::S7 => "S7 class",
        };
        Some((
            format!("{role} {}", receiver.class.identity.name),
            self.installed.constructor(receiver),
            receiver
                .class
                .help
                .as_ref()
                .map(|help| ObjectHelp::new(receiver.package, help)),
        ))
    }

    /// Load only the installed package that owns this callable. The class
    /// registries inspect resident metadata to avoid loading every package.
    pub(crate) fn indexed_dispatch_system(
        &self,
        value: &SyntaxElement,
    ) -> Option<crate::rindex::schema::ObjectSystem> {
        let (_, generic) = self
            .installed
            .generic(self.index, self.path, self.root, self.model, value)?;
        Some(generic.system)
    }

    pub(crate) fn external_s7_implementations(&self, call: &CallExpr) -> Vec<OopId> {
        let Some(value) = call.base() else {
            return Vec::new();
        };
        let Some((package, generic)) = self
            .installed
            .generic(self.index, self.path, self.root, self.model, &value)
        else {
            return Vec::new();
        };
        if generic.system != crate::rindex::schema::ObjectSystem::S7 {
            return Vec::new();
        }
        self.index.s7_external_methods(
            self.path,
            self.root,
            self.model,
            call,
            generic
                .identity
                .package
                .as_deref()
                .unwrap_or(package.package.as_str()),
            &generic.identity.name,
        )
    }

    pub(crate) fn generic_hover(
        &self,
        value: &SyntaxElement,
    ) -> Option<(String, Option<CallableSignature>, Option<ObjectHelp>)> {
        let (package, generic) = self
            .installed
            .generic(self.index, self.path, self.root, self.model, value)?;
        Some((
            format!("{:?} generic {}", generic.system, generic.identity.name),
            generic.formals.as_deref().map(from_formals),
            generic
                .help
                .as_ref()
                .map(|help| ObjectHelp::new(package, help)),
        ))
    }

    pub(crate) fn method_hover(
        &self,
        value: &SyntaxElement,
    ) -> Option<(String, Option<CallableSignature>, Option<ObjectHelp>)> {
        let (package, method) = self
            .installed
            .method(self.index, self.path, self.root, self.model, value)?;
        let mut label = format!("{:?} method for `{}`", method.system, method.generic.name);
        if let Some(dispatch) = &method.dispatch {
            let classes = dispatch
                .iter()
                .map(|parameter| {
                    parameter
                        .class
                        .as_ref()
                        .map(installed::object_type_label)
                        .unwrap_or_else(|| "?".into())
                })
                .collect::<Vec<_>>()
                .join(", ");
            label.push_str(&format!(" ({classes})"));
        }
        Some((
            label,
            method.formals.as_deref().map(from_formals),
            method
                .help
                .as_ref()
                .map(|help| ObjectHelp::new(package, help)),
        ))
    }

    pub(crate) fn members(&self, value: &SyntaxElement, at: bool) -> Option<MemberSurface> {
        let evidence = self.receivers(value);
        if evidence.candidates.is_empty() {
            return None;
        }
        let unique = evidence.candidates.len() == 1 && !evidence.unknown;
        let mut result = MemberSurface {
            complete: unique,
            ..MemberSurface::default()
        };
        for receiver in evidence.candidates {
            let surface = match &receiver {
                EditorReceiver::Source(receiver) => self.source_members(receiver, at),
                EditorReceiver::Installed(receiver) => self.installed.members(*receiver, at),
            };
            result.complete &= surface.complete;
            for (name, mut member) in surface.members {
                if !unique {
                    member.signature = None;
                    member.declared_type = None;
                    member.help = None;
                }
                result.insert(name, member);
            }
        }
        Some(result)
    }

    fn source_members(&self, receiver: &ClassReceiver, at: bool) -> MemberSurface {
        let index = self.index;
        let system = receiver.class.key.system;
        let generator = receiver.access == ClassAccess::Generator;
        let mut surface = MemberSurface {
            complete: true,
            ..MemberSurface::default()
        };
        match (system, at, generator) {
            (ClassSystem::S7, true, false) => {
                let Some(api) = index.s7_api(&receiver.class) else {
                    return MemberSurface::default();
                };
                surface.complete = api.complete;
                for property in api.properties {
                    let mut member = EditorMember::new(MemberKind::Property);
                    member.declared_type = property.declared_type(index);
                    surface.members.insert(property.name, member);
                }
            }
            (ClassSystem::S4, true, false) => {
                let Some(slots) = index.s4_slots(&receiver.class) else {
                    return MemberSurface::default();
                };
                surface.complete = slots.complete;
                for (name, entries) in slots.entries {
                    let mut member = EditorMember::new(MemberKind::Slot);
                    member.declared_type = entries
                        .first()
                        .filter(|first| {
                            entries
                                .iter()
                                .all(|slot| slot.declared_type == first.declared_type)
                        })
                        .and_then(|slot| declared_type(&slot.declared_type));
                    surface.members.insert(name, member);
                }
            }
            (ClassSystem::RefClass, _, _) if !at || !generator => {
                let Some(api) = index.reference_receiver_api(receiver) else {
                    return MemberSurface::default();
                };
                let members = if at {
                    api.slots
                } else if generator {
                    api.generator
                } else {
                    api.instance
                };
                surface.complete = members.complete;
                for (name, entries) in members.entries {
                    for entry in entries {
                        let mut member = EditorMember::new(entry.kind);
                        member.declared_type = entry.declared_type.as_ref().and_then(declared_type);
                        if surface.complete && receiver.exact && entry.kind == MemberKind::Method {
                            member.signature = if generator && name == "new" {
                                api.initializer.clone()
                            } else {
                                entry.signature
                            };
                        }
                        surface.insert(name.clone(), member);
                    }
                }
            }
            (ClassSystem::R6, false, _) => {
                let Some(api) = index.r6_api_at(&receiver.class, receiver.point.as_ref()) else {
                    return MemberSurface::default();
                };
                let members = api.members(receiver.access);
                surface.complete = members.complete;
                for (name, entries) in &members.entries {
                    for entry in entries {
                        let mut member = EditorMember::new(entry.kind);
                        if surface.complete && receiver.exact && entry.kind == MemberKind::Method {
                            member.signature = if generator && name == "new" {
                                api.initializer.clone()
                            } else {
                                entry.signature.clone()
                            };
                        }
                        surface.insert(name.clone(), member);
                    }
                }
            }
            _ => {}
        }
        let label = match system {
            ClassSystem::S7 => "S7",
            ClassSystem::S4 => "S4",
            ClassSystem::RefClass => "Reference Class",
            ClassSystem::R6 => "R6",
        };
        for member in surface.members.values_mut() {
            if member.origin.is_none() {
                member.origin = Some(label.into());
            }
        }
        surface
    }

    pub(crate) fn callable(&self, call: &CallExpr) -> Option<EditorCallable> {
        let base = call.base()?;
        if let Some(binary) = base.as_node().cloned().and_then(BinaryExpr::cast)
            && matches!(binary.op_kind(), Some(SyntaxKind::DOLLAR | SyntaxKind::AT))
        {
            let name = resolve::namespace_name(binary.rhs()?)?;
            let member = self
                .members(&binary.lhs()?, binary.op_kind() == Some(SyntaxKind::AT))?
                .members
                .remove(&name)?;
            return Some(EditorCallable {
                name,
                signature: member.signature?,
                help: member.help,
                notes: Vec::new(),
            });
        }
        if let Some(constructor) = self.constructor(call, &base) {
            return Some(constructor);
        }
        if let Some(signature) = self
            .index
            .implicit_member_signature(self.path, self.root, self.model, &base)
        {
            return Some(EditorCallable {
                name: base.to_string(),
                signature,
                help: None,
                notes: Vec::new(),
            });
        }
        self.generic(call, &base)
    }

    pub(crate) fn is_formal_constructor(&self, call: &CallExpr) -> bool {
        call_function_ref(self.model, self.root, call).is_some_and(|reference| {
            matches!(&reference, FunctionRef::Qualified { package, name } if package == "methods" && name == "new")
                || self.index.callee_identity(self.path, &reference)
                    .is_some_and(|origin| origin.package == "methods" && origin.name == "new")
        })
    }

    fn constructor(&self, call: &CallExpr, base: &SyntaxElement) -> Option<EditorCallable> {
        if self.is_formal_constructor(call) {
            let mut result = EditorCallable {
                name: base.to_string(),
                signature: simple_signature(&["Class", "..."]),
                help: None,
                notes: Vec::new(),
            };
            // The literal Class argument is usable while the closing paren is
            // still being typed, provided it matches unambiguously.
            if let Some(value) = incomplete_constructor_class(call) {
                if let Some(id) = self
                    .index
                    .s4_named_class(self.path, self.root, self.model, &value)
                {
                    result.notes = s4_slot_note(self.index, &id);
                } else if let Some(receiver) = self
                    .installed
                    .formal_class(self.index, self.path, self.root, self.model, &value, None)
                {
                    result.notes =
                        slot_note(self.installed.members(receiver, true).members.into_keys());
                    result.help = receiver
                        .class
                        .help
                        .as_ref()
                        .map(|help| ObjectHelp::new(receiver.package, help));
                }
            }
            return Some(result);
        }
        let (signature, help, notes) = match self.receivers(base).unique()? {
            EditorReceiver::Source(receiver)
                if receiver.exact && receiver.access == ClassAccess::Generator =>
            {
                let (signature, notes) = match receiver.class.key.system {
                    ClassSystem::S4 => (
                        simple_signature(&["..."]),
                        s4_slot_note(self.index, &receiver.class),
                    ),
                    ClassSystem::RefClass => (
                        self.index.reference_receiver_api(&receiver)?.initializer?,
                        Vec::new(),
                    ),
                    ClassSystem::S7 if self.index.s7_constructible(&receiver.class) => {
                        (self.index.s7_api(&receiver.class)?.constructor?, Vec::new())
                    }
                    _ => return None,
                };
                (signature, None, notes)
            }
            EditorReceiver::Installed(receiver)
                if receiver.access == ClassAccess::Generator
                    && matches!(
                        receiver.class.system,
                        crate::rindex::schema::ObjectSystem::Reference
                            | crate::rindex::schema::ObjectSystem::S7
                    ) =>
            {
                (
                    self.installed.constructor(receiver)?,
                    receiver
                        .class
                        .help
                        .as_ref()
                        .map(|help| ObjectHelp::new(receiver.package, help)),
                    Vec::new(),
                )
            }
            _ => return None,
        };
        Some(EditorCallable {
            name: base.to_string(),
            signature,
            help,
            notes,
        })
    }

    pub(crate) fn masks_indexed_callable(&self, call: &CallExpr) -> bool {
        let Some(base) = call.base() else {
            return false;
        };
        if base
            .as_node()
            .cloned()
            .and_then(BinaryExpr::cast)
            .is_some_and(|binary| binary.namespace_access().is_some())
        {
            return false;
        }
        call_function_ref(self.model, self.root, call).is_none()
            || self.index.installed_binding_masked(self.path, &base)
    }
}

fn declared_type(class: &DispatchClass) -> Option<String> {
    match class {
        DispatchClass::Formal(name) | DispatchClass::S3(name) => Some(name.clone()),
        DispatchClass::Any => Some("ANY".into()),
        _ => None,
    }
}

fn from_formals(formals: &[Formal]) -> CallableSignature {
    CallableSignature {
        parameters: formals
            .iter()
            .map(|formal| CallableParameter {
                name: formal.name.to_string(),
                default: formal.default.clone(),
            })
            .collect(),
    }
}

fn simple_signature(names: &[&str]) -> CallableSignature {
    CallableSignature {
        parameters: names
            .iter()
            .map(|name| CallableParameter {
                name: (*name).into(),
                default: None,
            })
            .collect(),
    }
}

fn incomplete_constructor_class(call: &CallExpr) -> Option<SyntaxElement> {
    let list = call.arg_list()?;
    let args: Vec<_> = list.args().collect();
    let names: Vec<_> = args.iter().map(|arg| arg.name()).collect();
    let matched = crate::semantic::checked_match_args_to_formals(&names, &["Class", "..."]).ok()?;
    args.iter()
        .zip(matched)
        .find(|(_, name)| *name == Some("Class"))?
        .0
        .value()
}

fn s4_slot_note(index: &ClassIndex, id: &ClassId) -> Vec<String> {
    index
        .s4_slots(id)
        .map_or_else(Vec::new, |slots| slot_note(slots.order.into_iter()))
}

fn slot_note(names: impl Iterator<Item = String>) -> Vec<String> {
    let names: Vec<_> = names.collect();
    if names.is_empty() {
        return Vec::new();
    }
    vec![format!("Declared slots: {}", names.join(", "))]
}

impl EditorRead<'_> {
    fn generic(&self, call: &CallExpr, base: &SyntaxElement) -> Option<EditorCallable> {
        let name = base.to_string();
        let can_select = call
            .syntax()
            .children_with_tokens()
            .any(|child| child.kind() == SyntaxKind::RPAREN)
            && !call
                .syntax()
                .parent()
                .and_then(BinaryExpr::cast)
                .is_some_and(|binary| {
                    binary
                        .op()
                        .is_some_and(|op| matches!(op.text(), "|>" | "%>%"))
                        && binary.rhs().and_then(SyntaxElement::into_node).as_ref()
                            == Some(call.syntax())
                });
        if let Some((generic, selected)) = self
            .index
            .s4_ordinary_call(self.path, self.root, self.model, call)
            && let Some(signature) = &generic.signature
        {
            return Some(EditorCallable {
                name,
                signature: signature.clone(),
                notes: selected.filter(|_| can_select).into_iter().collect(),
                help: None,
            });
        }
        if let Some(id) =
            self.index
                .s7_live_generic(self.path, self.root, self.model, &self.extracted, base)
        {
            let generic = self.index.generic(&id)?;
            if generic
                .uncertainty
                .contains(&OopUncertainty::DynamicSignature)
            {
                return None;
            }
            let signature = generic.signature.clone()?;
            let selected = can_select
                .then(|| {
                    self.index
                        .s7_dispatch_at(self.path, self.root, self.model, call)
                })
                .flatten()
                .filter(|dispatch| dispatch.complete && dispatch.generic == id)
                .and_then(|dispatch| self.index.method(&dispatch.selected?))
                .map(|method| method_note(&name, &method.dispatch));
            return Some(EditorCallable {
                name,
                signature,
                notes: selected.into_iter().collect(),
                help: None,
            });
        }
        let value = SyntaxElement::Node(call.syntax().clone());
        if let Some(dispatch) = self
            .index
            .s3_dispatch_at(self.path, self.root, self.model, &value)
            && !dispatch
                .uncertainty
                .contains(&S3DispatchUncertainty::AmbiguousGeneric)
            && let S3GenericId::Declaration(id) = &dispatch.generic
        {
            let signature = self.index.generic(id)?.signature.clone()?;
            let selected = dispatch
                .selected
                .as_ref()
                .filter(|_| can_select && dispatch.uncertainty.is_empty())
                .and_then(|selected| match &selected.class {
                    ClassName::Known(class) => Some(format!(
                        "Selected method: `{}.{class}`",
                        dispatch.dispatch_name
                    )),
                    _ => None,
                });
            return Some(EditorCallable {
                name,
                signature,
                notes: selected.into_iter().collect(),
                help: None,
            });
        }
        let (package, generic) = self
            .installed
            .generic(self.index, self.path, self.root, self.model, base)?;
        Some(EditorCallable {
            name,
            signature: from_formals(generic.formals.as_deref()?),
            notes: Vec::new(),
            help: generic
                .help
                .as_ref()
                .map(|help| ObjectHelp::new(package, help)),
        })
    }
}

pub(super) fn method_note(name: &str, arguments: &[DispatchArgument]) -> String {
    format!(
        "Selected method: `{}({})`",
        name,
        dispatch_labels(arguments, |_| None).join(", ")
    )
}

pub(crate) fn dispatch_labels(
    arguments: &[DispatchArgument],
    local_name: impl Fn(&ClassKey) -> Option<String>,
) -> Vec<String> {
    arguments
        .iter()
        .map(|argument| match &argument.class {
            DispatchClass::Missing => "missing".to_string(),
            DispatchClass::S7(spec) => s7_dispatch_label(spec, &local_name),
            DispatchClass::Generator(reference) => generator_label(reference, &local_name),
            class => declared_type(class).unwrap_or_else(|| "?".into()),
        })
        .collect()
}

fn s7_dispatch_label(
    spec: &S7ClassSpec,
    local_name: &impl Fn(&ClassKey) -> Option<String>,
) -> String {
    match spec {
        S7ClassSpec::Builtin(
            FunctionRef::Bare { name, .. } | FunctionRef::Qualified { name, .. },
        ) => name.clone(),
        S7ClassSpec::Generator(reference) => generator_label(reference, &local_name),
        S7ClassSpec::S3 { classes, .. } => classes.join(" / "),
        S7ClassSpec::S4 {
            package: Some(package),
            name,
            ..
        }
        | S7ClassSpec::External { package, name, .. } => format!("{package}::{name}"),
        S7ClassSpec::S4 { name, .. } => name.clone(),
        S7ClassSpec::Union { classes, .. } => classes
            .iter()
            .map(|class| s7_dispatch_label(class, local_name))
            .collect::<Vec<_>>()
            .join(" | "),
        S7ClassSpec::Null => "NULL".into(),
        _ => "?".into(),
    }
}

fn generator_label(
    reference: &GeneratorRef,
    local_name: &impl Fn(&ClassKey) -> Option<String>,
) -> String {
    match reference {
        GeneratorRef::Qualified { package, name } => format!("{package}::{name}"),
        GeneratorRef::Internal { package, name } => format!("{package}:::{name}"),
        GeneratorRef::Free(name) => name.clone(),
        GeneratorRef::Local(key) => local_name(key).unwrap_or_else(|| "?".into()),
        GeneratorRef::Unknown => "?".into(),
    }
}
