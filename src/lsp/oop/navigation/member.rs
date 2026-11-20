//! Member locations use the effective API of a resolved source receiver.

use super::*;
use crate::ast::BinaryExpr;
use crate::project::classes::{
    ClassAccess, ClassName, ClassSystem, MemberKind, MemberVisibility, OopPoint,
};

pub(super) fn member_at(
    index: &ClassIndex,
    path: &Path,
    root: &SyntaxNode,
    model: &SemanticModel,
    site: &OopSourceSite,
) -> Option<OopId> {
    let OopTarget::Member { name, slot, .. } = &site.site.target else {
        return None;
    };
    let node = root.covering_element(site.site.range).parent()?;
    let receiver = if site.required_callee.is_some() {
        let call = node.ancestors().find_map(crate::ast::CallExpr::cast)?;
        if let Some((object, _, _)) =
            crate::project::classes::slot_call(index, path, root, model, &call)
        {
            index.class_receiver(path, root, model, &object)?
        } else {
            let property = index.s7_property_use(
                path,
                root,
                model,
                &crate::syntax::SyntaxElement::Node(call.syntax().clone()),
            )?;
            if !property.complete || property.names.as_slice() != [name.as_str()] {
                return None;
            }
            property.receiver
        }
    } else {
        let binary = BinaryExpr::cast(node)?;
        index.class_receiver(path, root, model, &binary.lhs()?)?
    };
    if receiver.class.key.system == ClassSystem::RefClass {
        let api = index.reference_receiver_api(&receiver)?;
        let members = match (*slot, receiver.access) {
            (true, ClassAccess::Generator) => return None,
            (true, _) => api.slots,
            (false, ClassAccess::Generator) => api.generator,
            (false, _) => api.instance,
        };
        let [member] = members.entries.get(name)?.as_slice() else {
            return None;
        };
        return member.declaration.clone().filter(|_| members.complete);
    }
    effective_member(
        index,
        &receiver.class,
        receiver.access,
        receiver.point.as_ref(),
        name,
        *slot,
    )
}

fn effective_member(
    index: &ClassIndex,
    class: &ClassId,
    access: ClassAccess,
    point: Option<&OopPoint>,
    name: &str,
    slot: bool,
) -> Option<OopId> {
    match (class.key.system, slot, access) {
        (ClassSystem::R6, false, access) => {
            let api = index.r6_api_at(class, point)?;
            let members = api.members(access);
            let [member] = members.entries.get(name)?.as_slice() else {
                return None;
            };
            member.declaration.clone().filter(|_| members.complete)
        }
        (ClassSystem::RefClass, _, ClassAccess::Public) => {
            let api = index.reference_api_at(class, point)?;
            let members = if slot { api.slots } else { api.instance };
            let [member] = members.entries.get(name)?.as_slice() else {
                return None;
            };
            member.declaration.clone().filter(|_| members.complete)
        }
        (ClassSystem::S4, true, ClassAccess::Public) => {
            let slots = index.s4_slots(class)?;
            let [slot] = slots.entries.get(name)?.as_slice() else {
                return None;
            };
            slot.declaration.clone().filter(|_| slots.complete)
        }
        (ClassSystem::S7, true, ClassAccess::Public) => {
            let api = index.s7_api(class)?;
            if !api.complete {
                return None;
            }
            api.properties
                .into_iter()
                .find(|entry| entry.name == name)?
                .declaration
        }
        _ => None,
    }
}

/// Traverse confirmed inheritance edges and keep the effective declaration at
/// each descendant. Inherited entries share the original declaration identity.
pub(super) fn overrides(index: &ClassIndex, id: &OopId) -> Vec<OopId> {
    let Some(member) = index.member(id) else {
        return Vec::new();
    };
    let (Some(key), ClassName::Known(name)) = (&member.class, &member.name) else {
        return Vec::new();
    };
    let access = match member.visibility {
        MemberVisibility::Public => ClassAccess::Public,
        MemberVisibility::Private if key.system == ClassSystem::R6 => ClassAccess::Private,
        _ => return Vec::new(),
    };
    let slot = matches!(member.kind, MemberKind::Slot | MemberKind::Property);
    let class = ClassId {
        path: id.path.clone(),
        key: key.clone(),
    };
    let mut visited = std::collections::BTreeSet::from([class.clone()]);
    let mut pending = vec![class];
    let mut result = Vec::new();
    while let Some(parent) = pending.pop() {
        for child in index.subtypes.get(&parent).into_iter().flatten() {
            if !visited.insert(child.clone()) {
                continue;
            }
            pending.push(child.clone());
            if let Some(candidate) = effective_member(index, child, access, None, name, slot)
                && candidate != *id
                && index.member(&candidate).is_some_and(|member| {
                    member
                        .class
                        .as_ref()
                        .is_none_or(|key| candidate.path == child.path && *key == child.key)
                })
                && !result.contains(&candidate)
            {
                result.push(candidate);
            }
        }
    }
    result
}
