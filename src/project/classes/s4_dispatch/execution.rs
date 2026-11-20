//! Evidence at a live dispatch call. The shared registry retains only event
//! order and identities, so moving a query never changes that registry.

use super::*;
use crate::ast::ParenExpr;
use crate::project::classes::s4::{S4NamespaceLoad, S4NamespaceLoadKind};
use crate::project::source::{
    LoadingCallKind, SourceEdge, SourceTarget, loading_calls, source_edge, unconditional_statements,
};
use crate::project::source_context::SourceContext;
use crate::semantic::enclosure::Availability::{self, Absent, Present, Unknown};
use std::cmp::Ordering;

#[derive(Debug, Clone, Default)]
pub(super) struct S4Execution {
    pub oop: BTreeSet<OopKey>,
    pub classes: BTreeSet<ClassKey>,
    pub bridges: BTreeSet<ClassKey>,
    pub relations: BTreeSet<usize>,
    owners: BTreeSet<ClassOwner>,
    definite_sources: BTreeSet<PathBuf>,
    possible_sources: BTreeSet<PathBuf>,
    pub source_incomplete: bool,
}

pub(in crate::project::classes) struct S4SourceSite {
    range: TextRange,
    function: Option<TextRange>,
    edge: Option<SourceEdge>,
}

impl S4Execution {
    pub(super) fn source_activity(&self, path: &Path) -> Availability {
        if self.definite_sources.contains(path) {
            Present
        } else if self.possible_sources.contains(path) {
            Unknown
        } else {
            Absent
        }
    }
}

impl ClassIndex {
    pub(super) fn s4_execution_at(
        &self,
        path: &Path,
        site: &S4CallSite,
        extracted: &Extracted,
        sources: &[S4SourceSite],
    ) -> S4Execution {
        let before = |range: TextRange| range.end() <= site.range.start();
        let mut execution = S4Execution {
            oop: extracted
                .facts
                .oop
                .events
                .iter()
                .zip(&extracted.oop_event_ranges)
                .filter(|(_, range)| before(**range))
                .filter_map(|(event, _)| match event {
                    OopEvent::Generic { target, .. }
                    | OopEvent::Method(target)
                    | OopEvent::S7Method(target)
                    | OopEvent::RemoveS4Method(target) => Some(target.clone()),
                    _ => None,
                })
                .collect(),
            classes: extracted
                .facts
                .declarations
                .iter()
                .zip(&extracted.locations)
                .filter(|(_, location)| before(location.call_range))
                .map(|(def, _)| def.key.clone())
                .collect(),
            bridges: extracted
                .s4_old_class_sites
                .iter()
                .filter(|(_, range)| before(**range))
                .map(|(key, _)| key.clone())
                .collect(),
            relations: extracted
                .facts
                .oop
                .events
                .iter()
                .zip(&extracted.oop_event_ranges)
                .filter_map(|(event, range)| match event {
                    OopEvent::S4Relation(ordinal) if before(*range) => Some(*ordinal),
                    _ => None,
                })
                .collect(),
            owners: extracted
                .owners
                .iter()
                .filter(|(range, _)| before(**range))
                .map(|(_, owner)| owner.clone())
                .collect(),
            ..S4Execution::default()
        };
        let sources: Vec<_> = sources
            .iter()
            .filter(|source| source.function == site.function && before(source.range))
            .collect();
        if sources.is_empty() {
            return execution;
        }
        execution.source_incomplete = sources.iter().any(|source| source.edge.is_none());
        let graph = self.source_graph();
        let mut definite = None::<BTreeSet<_>>;
        for directory in graph.directories(path) {
            let mut in_directory = BTreeSet::new();
            for edge in sources.iter().filter_map(|source| source.edge.as_ref()) {
                let resolved = edge.key().resolve(directory);
                match resolved.target {
                    SourceTarget::Path(path) if !resolved.local => {
                        let closure = graph.closure(SourceContext {
                            path,
                            directory: resolved.directory,
                        });
                        execution.source_incomplete |= closure.incomplete;
                        in_directory.extend(closure.paths);
                    }
                    _ => execution.source_incomplete = true,
                }
            }
            execution
                .possible_sources
                .extend(in_directory.iter().cloned());
            match &mut definite {
                Some(definite) => definite.retain(|path| in_directory.contains(path)),
                None => definite = Some(in_directory),
            }
        }
        if let Some(definite) = definite {
            execution.definite_sources = definite;
        } else {
            execution.source_incomplete = true;
        }
        execution
    }

    pub(super) fn s4_file_activity(
        &self,
        context: &S4DispatchContext,
        path: &Path,
        loaded: &BTreeSet<PathBuf>,
    ) -> Availability {
        if path == context.path {
            return Present;
        }
        if let Some(root) = self.file(path).and_then(|file| file.package_root.as_ref()) {
            if self
                .file(&context.path)
                .and_then(|file| file.package_root.as_ref())
                == Some(root)
            {
                return if context.deferred {
                    Present
                } else {
                    match self.package_file_order(root, path, &context.path) {
                        Some(std::cmp::Ordering::Less) => Present,
                        Some(_) => Absent,
                        None => Unknown,
                    }
                };
            }
            if loaded.contains(root) {
                return Present;
            }
        }
        if let Some(execution) = &context.execution {
            let source = execution.source_activity(path);
            if source != Absent || !context.deferred {
                return source;
            }
        }
        if self.sees_file(&context.path, path) {
            Present
        } else {
            Absent
        }
    }

    pub(super) fn s4_site_activity(
        &self,
        context: &S4DispatchContext,
        path: &Path,
        owner: &ClassOwner,
        file: Availability,
        preceding: impl FnOnce(&S4Execution) -> bool,
    ) -> Availability {
        if file == Absent {
            return Absent;
        }
        if owner.0.is_empty() {
            if path == context.path
                && !context.deferred
                && context
                    .execution
                    .as_ref()
                    .is_some_and(|execution| !preceding(execution))
            {
                return Absent;
            }
            return file;
        }
        if path == context.path && context.owner.0.starts_with(&owner.0) {
            return if context
                .execution
                .as_ref()
                .is_some_and(|execution| !preceding(execution))
            {
                Absent
            } else {
                file
            };
        }
        if path == context.path
            && !context.deferred
            && context
                .execution
                .as_ref()
                .is_some_and(|execution| !execution.owners.contains(owner))
        {
            return Absent;
        }
        // This closure can have installed metadata in its top-level environment.
        Unknown
    }

    pub(super) fn s4_event_order(
        &self,
        (left, left_event): (&Path, usize),
        (right, right_event): (&Path, usize),
    ) -> Option<Ordering> {
        if left == right {
            return Some(left_event.cmp(&right_event));
        }
        let root = self.file(left)?.package_root.as_ref()?;
        if self.file(right)?.package_root.as_ref()? != root {
            return None;
        }
        self.package_file_order(root, left, right)
    }
}

pub(super) fn source_sites(root: &SyntaxNode, model: &SemanticModel) -> Vec<S4SourceSite> {
    let top_level: std::collections::HashSet<_> = unconditional_statements(root)
        .into_iter()
        .map(|statement| statement.text_range())
        .collect();
    let mut proven: HashMap<_, _> = loading_calls(root, model, LoadingCallKind::Unshadowed)
        .filter(|call| {
            top_level.contains(&call.syntax().text_range())
                || (unconditional(call.syntax())
                    && enclosing_function(call.syntax()).is_some()
                    && crate::project::source_env::source_destination(call)
                        == crate::project::source_env::Environment::Global)
        })
        .map(|call| {
            let edge = source_edge(&call, None);
            (edge.range, edge)
        })
        .collect();
    loading_calls(root, model, LoadingCallKind::Possible)
        .filter(|call| !crate::semantic::builder::is_quoted(call.syntax()))
        .map(|call| {
            let range = call.syntax().text_range();
            S4SourceSite {
                range,
                function: enclosing_function(call.syntax()),
                edge: proven.remove(&range),
            }
        })
        .collect()
}

fn enclosing_function(node: &SyntaxNode) -> Option<TextRange> {
    node.ancestors()
        .find_map(FunctionExpr::cast)
        .map(|function| function.syntax().text_range())
}

pub(in crate::project::classes) fn package_load_sites(
    root: &SyntaxNode,
    model: &SemanticModel,
    include_bodies: bool,
) -> Vec<S4PackageLoadSite> {
    root.descendants()
        .filter(|node| matches!(node.kind(), SyntaxKind::CALL_EXPR | SyntaxKind::BINARY_EXPR))
        .filter_map(|node| {
            let function = enclosing_function(&node);
            if !include_bodies && function.is_some() {
                return None;
            }
            let range = node.text_range();
            let load = match node.kind() {
                SyntaxKind::CALL_EXPR => package_load_call(root, model, CallExpr::cast(node)?)?,
                SyntaxKind::BINARY_EXPR => namespace_access_load(BinaryExpr::cast(node)?)?,
                _ => return None,
            };
            Some(S4PackageLoadSite {
                range,
                function,
                load,
            })
        })
        .collect()
}

fn namespace_access_load(binary: BinaryExpr) -> Option<S4NamespaceLoad> {
    let access = binary.namespace_access()?;
    if matches!(access.package.as_str(), "base" | "methods") {
        return None;
    }
    // The head of a call is evaluated before the call. Arguments can stay
    // unforced, so a qualified read in an argument supplies no load proof.
    let mut expression = binary.syntax().clone();
    while let Some(parent) = expression.parent() {
        let eager_parent = ParenExpr::cast(parent.clone()).is_some()
            || CallExpr::cast(parent.clone()).is_some_and(|call| {
                call.base()
                    .is_some_and(|base| base.text_range() == expression.text_range())
            });
        if !eager_parent {
            break;
        }
        expression = parent;
    }
    if expression
        .parent()
        .and_then(AssignmentExpr::cast)
        .is_some_and(|assignment| {
            assignment
                .target_element()
                .is_some_and(|target| target.text_range() == expression.text_range())
        })
    {
        return None;
    }
    let unconditional = unconditional(&expression);
    if !unconditional
        && !expression
            .ancestors()
            .skip(1)
            .take_while(|node| !matches!(node.kind(), SyntaxKind::ROOT | SyntaxKind::FUNCTION_EXPR))
            .all(|node| {
                matches!(
                    node.kind(),
                    SyntaxKind::BLOCK_EXPR
                        | SyntaxKind::ASSIGNMENT_EXPR
                        | SyntaxKind::IF_EXPR
                        | SyntaxKind::FOR_EXPR
                        | SyntaxKind::WHILE_EXPR
                        | SyntaxKind::REPEAT_EXPR
                )
            })
    {
        return None;
    }
    Some(S4NamespaceLoad {
        package: Some(access.package.to_string()),
        unconditional,
        kind: S4NamespaceLoadKind::Access,
    })
}

fn package_load_call(
    root: &SyntaxNode,
    model: &SemanticModel,
    call: CallExpr,
) -> Option<S4NamespaceLoad> {
    if crate::semantic::builder::is_quoted(call.syntax()) {
        return None;
    }
    let callee = call_function_ref(model, root, &call);
    let name = match &callee {
        Some(FunctionRef::Qualified { package, name }) if package == "base" => name.clone(),
        Some(FunctionRef::Qualified { .. }) => return None,
        Some(FunctionRef::Bare { name, .. }) => name.clone(),
        _ => unbacktick(&call.callee_name()?).to_owned(),
    };
    let formals: &[_] = match name.as_str() {
        "library" | "require" => attachment_formals(&name)?,
        "requireNamespace" => &["package", "...", "quietly"],
        "loadNamespace" => &[
            "package",
            "lib.loc",
            "keep.source",
            "partial",
            "versionCheck",
            "keep.parse.data",
        ],
        _ => return None,
    };
    let args = matched_args(&call, formals);
    let value = |name| args.as_ref().and_then(|args| argument(args, name));
    if name == "library" && args.is_some() && value("package").is_none() {
        return None;
    }
    let package = value("package").and_then(|package| {
        let token = package.into_token()?;
        match token.kind() {
            SyntaxKind::STRING => crate::semantic::decode_string_literal(token.text()),
            SyntaxKind::IDENT
                if matches!(name.as_str(), "library" | "require")
                    && value("character.only")
                        .is_none_or(|value| constant_atom(&value, "FALSE")) =>
            {
                Some(unbacktick(token.text()).to_owned())
            }
            _ => None,
        }
    });
    let unconditional = unconditional(call.syntax())
        && value("lib.loc").is_none_or(|value| constant_atom(&value, "NULL"))
        && value("partial").is_none_or(|value| constant_atom(&value, "FALSE"));
    let attach_depends = if matches!(name.as_str(), "library" | "require") {
        match value("attach.required") {
            None => Some(value("include.only").is_none()),
            Some(value) if constant_atom(&value, "TRUE") => Some(true),
            Some(value) if constant_atom(&value, "FALSE") => Some(false),
            _ => None,
        }
    } else {
        Some(false)
    };
    Some(S4NamespaceLoad {
        package,
        unconditional,
        kind: S4NamespaceLoadKind::Call {
            callee,
            attach_depends,
        },
    })
}

impl ClassIndex {
    pub(in crate::project::classes) fn s4_loader_activity(
        &self,
        path: &Path,
        load: &S4NamespaceLoad,
    ) -> Availability {
        let activity = match &load.kind {
            S4NamespaceLoadKind::Access => Present,
            S4NamespaceLoadKind::Call { callee, .. } => match callee {
                Some(FunctionRef::Qualified { package, .. }) if package == "base" => Present,
                Some(reference) => self.callee_availability(path, reference),
                None => Unknown,
            },
        };
        if activity == Present && !load.unconditional {
            Unknown
        } else {
            activity
        }
    }

    pub(super) fn s4_add_loaded_package(
        &self,
        context: &mut S4DispatchContext,
        package: Option<&str>,
    ) {
        context.namespace_load_incomplete |=
            !self.s4_load_workspace_root(package, &mut context.loaded_packages);
    }

    fn s4_load_workspace_root(&self, package: Option<&str>, roots: &mut BTreeSet<PathBuf>) -> bool {
        match package {
            Some("base" | "methods") => true,
            Some(package) => self.s4_workspace_root(package).is_some_and(|root| {
                roots.insert(root.clone());
                true
            }),
            None => false,
        }
    }

    pub(super) fn s4_record_load(&self, context: &mut S4DispatchContext, load: &S4NamespaceLoad) {
        context.namespace_load_incomplete |= !self.s4_load_into(
            load,
            &mut context.loaded_packages,
            &mut context.attached_packages,
        );
    }

    fn s4_load_into(
        &self,
        load: &S4NamespaceLoad,
        loaded: &mut BTreeSet<PathBuf>,
        attached: &mut BTreeSet<PathBuf>,
    ) -> bool {
        let package = load.package.as_deref();
        match load.attach_depends() {
            Some(true) => self.s4_load_workspace_root(package, attached),
            attach => self.s4_load_workspace_root(package, loaded) && attach.is_some(),
        }
    }

    /// Sources and package files that have run can themselves load more
    /// namespaces. They contribute only their range-free, file-scope loads.
    pub(super) fn s4_dispatch_packages(
        &self,
        context: &S4DispatchContext,
        explicit: &BTreeSet<PathBuf>,
    ) -> (BTreeSet<PathBuf>, bool) {
        let mut roots = explicit.clone();
        let mut attached = context.attached_packages.clone();
        let mut processed = BTreeSet::new();
        let mut complete = true;
        loop {
            let (loaded, scopes_complete) =
                self.s4_loaded_packages(&context.path, &roots, &attached);
            complete &= scopes_complete;
            roots.clone_from(&loaded);
            let attachment_count = attached.len();
            for file in self.oop_files().filter(|file| {
                !file.facts.s4.namespace_loads.is_empty() || !self.class_file_complete(&file.path)
            }) {
                if file.path == context.path
                    && (context.execution.is_some() || !context.deferred || context.continuation)
                {
                    continue;
                }
                let activity = self.s4_file_activity(context, &file.path, &loaded);
                if activity == Absent || !processed.insert(&file.path) {
                    continue;
                }
                complete &= self.class_file_complete(&file.path);
                for load in &file.facts.s4.namespace_loads {
                    match self.s4_loader_activity(&file.path, load) {
                        Absent => {}
                        Present if activity == Present => {
                            complete &= self.s4_load_into(load, &mut roots, &mut attached);
                        }
                        _ => complete = false,
                    }
                }
            }
            if roots == loaded && attached.len() == attachment_count {
                return (roots, complete);
            }
        }
    }
}
