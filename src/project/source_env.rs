//! Static source destinations and ordered reads. Programs contain names and
//! structural order, never byte offsets or evaluated R values.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use rowan::TextRange;
use rowan::ast::AstNode as _;

use crate::ast::{
    AssignmentExpr, BinaryExpr, BlockExpr, CallExpr, FunctionExpr, HasArgList, IfExpr, ParenExpr,
    Subset2Expr,
};
use crate::semantic::symbols::unbacktick;
use crate::semantic::{ScopeKind, SemanticModel};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

use super::source::{SourceEdgeKey, source_call, source_edge, source_file_value};

mod replay;
pub(crate) use replay::SourceEnvironments;

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum Environment {
    Global,
    Current,
    Named(String),
    Fresh(Box<Environment>),
    Base,
    Unknown,
    Builtin(String, Box<Environment>),
}

pub(crate) fn environment(value: &SyntaxElement) -> Environment {
    if let Some(paren) = value.as_node().cloned().and_then(ParenExpr::cast) {
        return paren
            .inner()
            .as_ref()
            .map(environment)
            .unwrap_or(Environment::Unknown);
    }
    if let Some(token) = value.as_token() {
        return match token.text() {
            ".GlobalEnv" | "FALSE" => Environment::Global,
            "TRUE" => Environment::Current,
            "NULL" => Environment::Base,
            name if token.kind() == SyntaxKind::IDENT => {
                Environment::Named(unbacktick(name).into())
            }
            _ => Environment::Unknown,
        };
    }
    let Some(call) = value.as_node().cloned().and_then(CallExpr::cast) else {
        return Environment::Unknown;
    };
    if let Some(base) = call.base().and_then(SyntaxElement::into_node)
        && BinaryExpr::cast(base)
            .and_then(|binary| binary.namespace_access())
            .is_none_or(|access| access.package != "base")
    {
        return Environment::Unknown;
    }
    let args: Vec<_> = call
        .arg_list()
        .map(|args| args.args().collect())
        .unwrap_or_default();
    let result = match call.callee_name().as_deref().map(unbacktick) {
        Some("environment") if args.is_empty() => Environment::Current,
        Some("globalenv") if args.is_empty() => Environment::Global,
        Some("baseenv" | "emptyenv") if args.is_empty() => Environment::Base,
        Some("new.env") => {
            let mut parent = Environment::Current;
            for (index, arg) in args.iter().enumerate() {
                if arg.name().as_deref() == Some("parent") || (arg.name().is_none() && index == 1) {
                    parent = arg
                        .value()
                        .as_ref()
                        .map(environment)
                        .unwrap_or(Environment::Unknown);
                }
            }
            Environment::Fresh(Box::new(parent))
        }
        _ => Environment::Unknown,
    };
    if call.base().is_some_and(|base| base.as_token().is_some()) && result != Environment::Unknown {
        Environment::Builtin(
            call.callee_name()
                .expect("a recognized environment constructor")
                .to_string(),
            Box::new(result),
        )
    } else {
        result
    }
}

pub(crate) fn source_destination(call: &CallExpr) -> Environment {
    source_file_value(call)
        .1
        .as_ref()
        .map(environment)
        .unwrap_or_else(|| {
            if call.callee_name().as_deref() == Some("sys.source") {
                Environment::Base
            } else {
                Environment::Global
            }
        })
}

impl Environment {
    pub(crate) fn shares_file_scope(&self) -> bool {
        match self {
            Self::Global | Self::Current => true,
            Self::Builtin(_, environment) => environment.shares_file_scope(),
            _ => false,
        }
    }
}

/// An occurrence number among free reads of the same name. Whitespace and
/// edits to expression values leave these keys unchanged.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, salsa::SalsaValue)]
pub struct ReadKey {
    pub name: String,
    pub occurrence: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, salsa::SalsaValue)]
pub struct BindingKey {
    pub name: String,
    pub occurrence: u32,
}

pub(crate) fn binding_keys(model: &SemanticModel) -> HashMap<TextRange, BindingKey> {
    let mut counts: HashMap<&str, u32> = HashMap::new();
    model
        .bindings()
        .iter()
        .map(|binding| {
            let occurrence = counts.entry(binding.name.as_str()).or_default();
            let key = BindingKey {
                name: binding.name.to_string(),
                occurrence: *occurrence,
            };
            *occurrence += 1;
            (binding.def_range, key)
        })
        .collect()
}

pub(crate) fn read_keys(model: &SemanticModel) -> HashMap<TextRange, ReadKey> {
    let mut counts: HashMap<&str, u32> = HashMap::new();
    model
        .idents()
        .iter()
        .filter(|ident| model.resolve_local(ident).is_none())
        .map(|ident| {
            let occurrence = counts.entry(ident.name.as_str()).or_default();
            let key = ReadKey {
                name: ident.name.to_string(),
                occurrence: *occurrence,
            };
            *occurrence += 1;
            (ident.range, key)
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct SourceProgram(pub(crate) Vec<Event>);

impl SourceProgram {
    pub(crate) fn has_sources(&self) -> bool {
        self.0.iter().any(|event| match event {
            Event::Source { .. } => true,
            Event::Function { program, .. } => program.has_sources(),
            Event::Deferred(program) => program.has_sources(),
            Event::Branch(left, right) => left.has_sources() || right.has_sources(),
            _ => false,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub(crate) enum Event {
    Read {
        name: String,
        key: Option<ReadKey>,
    },
    Member {
        receiver: Environment,
        name: String,
    },
    UseOnly {
        name: String,
        gate: Option<String>,
    },
    Define {
        name: String,
        value: Environment,
        exported: bool,
        key: Option<BindingKey>,
    },
    Source {
        edge: SourceEdgeKey,
        destination: Environment,
        bare: Option<String>,
    },
    Function {
        id: usize,
        program: SourceProgram,
    },
    Branch(SourceProgram, SourceProgram),
    Deferred(SourceProgram),
}

pub fn collect_source_program(
    root: &SyntaxNode,
    base_dir: Option<&Path>,
    model: &SemanticModel,
) -> SourceProgram {
    let keys = read_keys(model);
    let idents: HashMap<_, _> = model
        .idents()
        .iter()
        .map(|ident| (ident.range, (ident.name.to_string(), ident.deferred)))
        .collect();
    let keys_by_binding = binding_keys(model);
    let bindings: HashMap<_, _> = model
        .bindings()
        .iter()
        .map(|binding| {
            (
                binding.def_range,
                (
                    binding.name.to_string(),
                    model.scope(binding.scope).kind == ScopeKind::File,
                    keys_by_binding[&binding.def_range].clone(),
                ),
            )
        })
        .collect();
    let functions = root
        .descendants()
        .filter(|node| FunctionExpr::can_cast(node.kind()))
        .enumerate()
        .map(|(id, node)| (node.text_range(), id))
        .collect();
    let collector = Collector {
        base_dir,
        keys,
        idents,
        bindings,
        functions,
        use_only: model.use_only_reads().iter().fold(
            HashMap::<TextRange, Vec<Event>>::new(),
            |mut events, read| {
                let event = Event::UseOnly {
                    name: read.name.to_string(),
                    gate: read.gate.as_ref().map(|(name, _)| name.to_string()),
                };
                events
                    .entry(read.range)
                    .or_default()
                    .push(if read.deferred {
                        Event::Deferred(SourceProgram(vec![event]))
                    } else {
                        event
                    });
                events
            },
        ),
    };
    let mut events = Vec::new();
    for element in root.children_with_tokens() {
        collector.walk(element, true, &mut events);
    }
    SourceProgram(events)
}

struct Collector<'a> {
    base_dir: Option<&'a Path>,
    keys: HashMap<TextRange, ReadKey>,
    idents: HashMap<TextRange, (String, bool)>,
    bindings: HashMap<TextRange, (String, bool, BindingKey)>,
    functions: HashMap<TextRange, usize>,
    use_only: HashMap<TextRange, Vec<Event>>,
}

impl Collector<'_> {
    fn elements(
        &self,
        elements: impl IntoIterator<Item = SyntaxElement>,
        effects: bool,
    ) -> SourceProgram {
        let mut events = Vec::new();
        for element in elements {
            self.walk(element, effects, &mut events);
        }
        SourceProgram(events)
    }

    fn walk(&self, element: SyntaxElement, effects: bool, events: &mut Vec<Event>) {
        let Some(node) = element.as_node() else {
            if let Some(reads) = self.use_only.get(&element.text_range()) {
                events.extend(reads.iter().cloned());
            }
            if let Some((name, deferred)) = self.idents.get(&element.text_range()) {
                let event = Event::Read {
                    name: name.clone(),
                    key: self.keys.get(&element.text_range()).cloned(),
                };
                events.push(if *deferred {
                    Event::Deferred(SourceProgram(vec![event]))
                } else {
                    event
                });
            }
            return;
        };
        if let Some(function) = FunctionExpr::cast(node.clone()) {
            let mut body = Vec::new();
            // Parameters shadow an enclosing helper environment or loaded name.
            for param in function.params() {
                body.push(Event::Define {
                    name: param.name.to_string(),
                    value: Environment::Unknown,
                    exported: false,
                    key: None,
                });
            }
            if let Some(element) = function.body() {
                self.walk(element, true, &mut body);
            }
            for formal in function.formals() {
                if let Some(default) = formal.default() {
                    self.walk(default, false, &mut body);
                }
            }
            events.push(Event::Function {
                id: self.functions[&node.text_range()],
                program: SourceProgram(body),
            });
            return;
        }
        if let Some(block) = BlockExpr::cast(node.clone()) {
            events.extend(self.elements(block.statements(), effects).0);
            return;
        }
        if let Some(assign) = AssignmentExpr::cast(node.clone()) {
            if let Some(value) = assign.value_element() {
                self.walk(value.clone(), effects, events);
                if let Some(target) = assign.target_element()
                    && target.as_node().is_some()
                {
                    // Computed targets read their indices and replacement
                    // functions even though they do not define a bare name.
                    self.walk(target, false, events);
                }
                if effects
                    && let Some(target) = assign.target_element()
                    && let Some((name, exported, key)) = self.bindings.get(&target.text_range())
                {
                    events.push(Event::Define {
                        name: name.clone(),
                        value: environment(&value),
                        exported: *exported,
                        key: Some(key.clone()),
                    });
                }
            }
            return;
        }
        if let Some(branch) = IfExpr::cast(node.clone()) {
            let condition = branch.condition_elements().unwrap_or_default();
            events.extend(self.elements(condition.clone(), effects).0);
            let meaningful: Vec<_> = condition
                .iter()
                .filter(|element| {
                    !matches!(
                        element.kind(),
                        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
                    )
                })
                .collect();
            if let [condition] = meaningful.as_slice()
                && let Some(taken) = literal_bool(condition)
            {
                let elements = if taken {
                    branch.then_elements()
                } else {
                    branch.else_elements()
                };
                events.extend(self.elements(elements.unwrap_or_default(), effects).0);
                return;
            }
            events.push(Event::Branch(
                self.elements(branch.then_elements().unwrap_or_default(), effects),
                self.elements(branch.else_elements().unwrap_or_default(), effects),
            ));
            return;
        }
        if let Some(call) = CallExpr::cast(node.clone()) {
            // Argument promises do not establish effects in the caller. This
            // also leaves quoted and computed loading calls unevaluated.
            events.extend(self.elements(node.children_with_tokens(), false).0);
            if effects {
                for arg in call.args() {
                    if let Some(value) = arg.value()
                        && let Some((name, exported, key)) = self.bindings.get(&value.text_range())
                    {
                        // The semantic builder already knows implicit bindings
                        // such as dataset names introduced by data().
                        events.push(Event::Define {
                            name: name.clone(),
                            value: Environment::Unknown,
                            exported: *exported,
                            key: Some(key.clone()),
                        });
                    }
                }
            }
            if effects && source_call(node).is_some() {
                events.push(Event::Source {
                    edge: source_edge(&call, self.base_dir).key(),
                    destination: source_destination(&call),
                    bare: call
                        .base()
                        .filter(|base| base.as_token().is_some())
                        .and_then(|_| call.callee_name())
                        .map(|name| name.to_string()),
                });
            }
            return;
        }
        if let Some(binary) = BinaryExpr::cast(node.clone()) {
            if matches!(binary.op_kind(), Some(SyntaxKind::AND2 | SyntaxKind::OR2)) {
                let constant = binary.lhs().as_ref().and_then(literal_bool);
                if let Some(left) = binary.lhs() {
                    self.walk(left, effects, events);
                }
                if let Some(left) = constant {
                    if left == (binary.op_kind() == Some(SyntaxKind::AND2)) {
                        events.extend(self.elements(binary.rhs(), effects).0);
                    }
                    return;
                }
                events.push(Event::Branch(
                    self.elements(binary.rhs(), effects),
                    SourceProgram(Vec::new()),
                ));
                return;
            }
            if binary.op_kind() == Some(SyntaxKind::DOLLAR)
                && let (Some(receiver), Some(member)) = (binary.lhs(), binary.rhs())
                && let Some(token) = member.as_token()
            {
                events.push(Event::Member {
                    receiver: environment(&receiver),
                    name: unbacktick(token.text())
                        .trim_matches(['\'', '"'])
                        .to_string(),
                });
            }
        }
        if let Some(subset) = Subset2Expr::cast(node.clone()) {
            let args: Vec<_> = subset.args().collect();
            if let (Some(receiver), [arg]) = (subset.base(), args.as_slice())
                && arg.name().is_none()
                && let Some(token) = arg.value().and_then(SyntaxElement::into_token)
                && token.kind() == SyntaxKind::STRING
                && let Some(name) = super::source::string_literal_content(token.text())
            {
                events.push(Event::Member {
                    receiver: environment(&receiver),
                    name: name.into(),
                });
            }
        }
        if matches!(
            node.kind(),
            SyntaxKind::FOR_EXPR | SyntaxKind::WHILE_EXPR | SyntaxKind::REPEAT_EXPR
        ) {
            events.push(Event::Branch(
                self.elements(node.children_with_tokens(), effects),
                SourceProgram(Vec::new()),
            ));
            return;
        }
        events.extend(self.elements(node.children_with_tokens(), effects).0);
    }
}

fn literal_bool(element: &SyntaxElement) -> Option<bool> {
    match element.as_token()?.text() {
        "TRUE" => Some(true),
        "FALSE" => Some(false),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, salsa::SalsaValue)]
pub struct SourceReads {
    pub reads: BTreeMap<ReadKey, SourceRead>,
    pub used_bindings: std::collections::BTreeSet<BindingKey>,
    pub introduced_names: std::collections::BTreeSet<String>,
    pub introductions_incomplete: bool,
}

impl SourceReads {
    pub(crate) fn used_ranges(
        &self,
        model: &SemanticModel,
    ) -> std::collections::HashSet<TextRange> {
        binding_keys(model)
            .into_iter()
            .filter_map(|(range, key)| self.used_bindings.contains(&key).then_some(range))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub enum SourceRead {
    Bound(PathBuf),
    Unbound,
    Unknown,
}
