//! Accessors are executable operations. Their formals describe the accessor,
//! never a stored value's type or the signature of a callable field.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessorArgument {
    Missing,
    AssignedValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberEffect {
    Stored,
    ReadOnly,
    InitializeOnce,
    InvokeAccessor {
        implementation: Option<OopId>,
        argument: AccessorArgument,
    },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberAccess {
    pub read: MemberEffect,
    pub write: MemberEffect,
}

impl ClassIndex {
    /// A source-level read or assignment can invoke an accessor. No accessor is
    /// evaluated, and its return value supplies no class or completion evidence.
    pub fn member_access(&self, receiver: &ClassReceiver, name: &str) -> Option<MemberAccess> {
        if receiver.access == ClassAccess::Generator {
            return None;
        }
        if receiver.class.key.system == ClassSystem::S4 {
            let slots = self.s4_slots(&receiver.class)?;
            slots.entries.get(name)?;
            return Some(MemberAccess {
                read: MemberEffect::Stored,
                write: MemberEffect::Stored,
            });
        }
        let (kind, declaration, write) = if receiver.class.key.system == ClassSystem::R6 {
            let api = self.r6_api_at(&receiver.class, receiver.point.as_ref())?;
            let members = api.members(receiver.access);
            let [member] = members.entries.get(name)?.as_slice() else {
                return None;
            };
            let write = if member.kind == MemberKind::Method {
                match self
                    .declarations
                    .get(&receiver.class)?
                    .declaration
                    .r6
                    .as_ref()?
                    .lock_objects
                {
                    Some(true) => MemberEffect::ReadOnly,
                    Some(false) => MemberEffect::Stored,
                    None => MemberEffect::Unknown,
                }
            } else {
                MemberEffect::Stored
            };
            (member.kind, member.declaration.clone(), write)
        } else {
            let api = self.reference_receiver_api(receiver)?;
            let [member] = api.instance.entries.get(name)?.as_slice() else {
                return None;
            };
            let locked_fields = self.reference_locked_fields(
                &receiver.class,
                receiver.point.as_ref(),
                &mut BTreeSet::new(),
            );
            let write =
                if member.kind == MemberKind::Method || matches!(name, ".self" | ".refClassDef") {
                    MemberEffect::ReadOnly
                } else if locked_fields.contains(name) {
                    MemberEffect::InitializeOnce
                } else if !api.instance.complete {
                    MemberEffect::Unknown
                } else {
                    MemberEffect::Stored
                };
            (member.kind, member.declaration.clone(), write)
        };
        if kind == MemberKind::ActiveBinding {
            let implementation = declaration.as_ref().and_then(|id| {
                Some(OopId {
                    path: id.path.clone(),
                    key: self.member(id)?.method.clone()?,
                })
            });
            return Some(MemberAccess {
                read: MemberEffect::InvokeAccessor {
                    implementation: implementation.clone(),
                    argument: AccessorArgument::Missing,
                },
                write: MemberEffect::InvokeAccessor {
                    implementation,
                    argument: AccessorArgument::AssignedValue,
                },
            });
        }
        Some(MemberAccess {
            read: if kind == MemberKind::Unknown {
                MemberEffect::Unknown
            } else {
                MemberEffect::Stored
            },
            write,
        })
    }
}
