//! Bounded evidence about an object's explicit S3 class attribute.
//!
//! These facts never declare classes or establish formal inheritance edges.

use std::path::Path;

use super::*;
use crate::ast::{BlockExpr, Expr, ParenExpr};

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum S3ClassEvidence {
    /// Exact attribute contents, in dispatch order (including duplicates).
    Explicit(Vec<String>),
    /// The attribute is absent. The object's implicit runtime class is unknown.
    NoExplicitClass,
    #[default]
    Unknown,
}

/// A stable file-scope callable with a recognizable S3 return. Helper origins
/// remain unconfirmed until the shared project query resolves them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S3Constructor {
    pub binding: String,
    pub classes: S3ClassEvidence,
    pub helpers: Vec<FunctionRef>,
    /// A known value type copies its attributes on replacement. Unknown values
    /// can be reference objects, so mutating an alias invalidates their evidence.
    pub by_value: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S3Facts {
    pub constructors: Vec<S3Constructor>,
    pub helpers: Vec<FunctionRef>,
}

impl ClassIndex {
    pub(super) fn s3_argument_evidence(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        call: &SyntaxElement,
        value: &SyntaxElement,
    ) -> S3ClassEvidence {
        self.s3_arguments_evidence(path, root, model, call, &[Some(value.clone())])
            .pop()
            .unwrap_or_default()
    }

    pub(super) fn s3_arguments_evidence(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        call: &SyntaxElement,
        values: &[Option<SyntaxElement>],
    ) -> Vec<S3ClassEvidence> {
        let mut lookup = Lookup::new(model, root, Some((self, path)));
        let mut env = Environment::default();
        let container = call
            .parent()
            .and_then(|node| node.ancestors().find_map(FunctionExpr::cast))
            .and_then(|function| function.body())
            .unwrap_or_else(|| root.clone().into());
        // The dispatching call has not forced its actuals yet. Start before
        // the call so it cannot invalidate evidence about its own arguments.
        lookup.prefix(&container, call.text_range(), &mut env, 0);
        // Eager operands share an evaluation environment in source order.
        for value in values {
            let operand = match value {
                Some(value) => lookup.eval(value, &mut env, 0),
                None => {
                    env.invalidate();
                    Value::default()
                }
            };
            env.retained.push(operand);
        }
        env.retained
            .into_iter()
            .map(|value| value.proof.classes)
            .collect()
    }

    pub(super) fn s3_generic_object(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        dispatch: &CallExpr,
        arguments: &BTreeMap<String, S3ClassEvidence>,
    ) -> S3ClassEvidence {
        let Some(object) = matched_args(dispatch, &["generic", "object"])
            .and_then(|args| argument(&args, "object"))
        else {
            return S3ClassEvidence::Unknown;
        };
        let Some(body) = dispatch
            .syntax()
            .ancestors()
            .find_map(FunctionExpr::cast)
            .and_then(|function| function.body())
        else {
            return S3ClassEvidence::Unknown;
        };
        let mut lookup = Lookup::new(model, root, Some((self, path)));
        let mut env = Environment::default();
        for (name, classes) in arguments {
            env.values.insert(
                name.clone(),
                Value {
                    proof: Proof {
                        classes: classes.clone(),
                        helpers: BTreeSet::new(),
                    },
                    ..Value::default()
                },
            );
        }
        lookup.prefix(&body, dispatch.syntax().text_range(), &mut env, 0);
        lookup.eval(&object, &mut env, 0).proof.classes
    }

    /// Recover only explicit class evidence, without executing R or inferring a
    /// class declaration from a constructor or dotted method binding.
    /// Evidence describes a value on normal return; it does not establish that
    /// the expression succeeds or infer the object's implicit runtime class.
    pub fn s3_class_evidence(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: &SyntaxElement,
    ) -> S3ClassEvidence {
        let mut lookup = Lookup::new(model, root, Some((self, path)));
        let mut env = Environment::default();
        let frame = value
            .parent()
            .and_then(|parent| parent.ancestors().find_map(FunctionExpr::cast));
        let container = frame
            .and_then(|function| function.body())
            .unwrap_or_else(|| root.clone().into());
        lookup.prefix(&container, value.text_range(), &mut env, 0);
        lookup.eval(value, &mut env, 0).proof.classes
    }
}

const HELPERS: &[&str] = &[
    "structure",
    "class",
    "class<-",
    "oldClass",
    "oldClass<-",
    "unclass",
    "c",
    "list",
    "character",
    "return",
    ".S3method",
    "registerS3method",
];

pub(super) fn project(model: &SemanticModel, root: &SyntaxNode) -> S3Facts {
    let calls = factories(model, root);
    let has_class_helpers = calls.iter().any(|(_, _, name)| class_helper(name));
    let helpers = calls
        .into_iter()
        .filter(|(_, _, name)| {
            HELPERS.contains(&name.as_str()) || matches!(name.as_str(), "UseMethod" | "NextMethod")
        })
        .map(|(_, reference, _)| reference)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut lookup = Lookup::new(model, root, None);
    let mut constructors = Vec::new();
    if !has_class_helpers {
        return S3Facts {
            helpers,
            constructors,
        };
    }
    for (i, binding) in model.bindings().iter().enumerate() {
        let id = BindingId::from_index(i);
        if !model.binding_is_file_scope(id) {
            continue;
        }
        let Some(function) = stable_value(model, root, id)
            .and_then(SyntaxElement::into_node)
            .and_then(FunctionExpr::cast)
        else {
            continue;
        };
        let value = lookup.function(&function, &mut Environment::default(), 0);
        let proof = value.proof;
        if proof.classes != S3ClassEvidence::Unknown
            && proof.helpers.iter().any(|helper| {
                possible_functions(helper)
                    .iter()
                    .any(|origin| class_helper(&origin.name))
            })
        {
            constructors.push(S3Constructor {
                binding: unbacktick(&binding.name).into(),
                classes: proof.classes,
                helpers: proof.helpers.into_iter().collect(),
                by_value: value.by_value,
            });
        }
    }
    S3Facts {
        constructors,
        helpers,
    }
}

fn class_helper(name: &str) -> bool {
    matches!(
        name,
        "structure" | "class" | "class<-" | "oldClass" | "oldClass<-" | "unclass"
    )
}

pub(super) fn generic_object_evidence(
    model: &SemanticModel,
    call: &CallExpr,
) -> Option<(S3ClassEvidence, Vec<FunctionRef>)> {
    let root = call.syntax().ancestors().last()?;
    let body = call
        .syntax()
        .ancestors()
        .find_map(FunctionExpr::cast)?
        .body()?;
    let object = argument(&matched_args(call, &["generic", "object"])?, "object")?;
    let mut lookup = Lookup::new(model, &root, None);
    let mut env = Environment::default();
    lookup.prefix(&body, call.syntax().text_range(), &mut env, 0);
    let proof = lookup.eval(&object, &mut env, 0).proof;
    (proof.classes != S3ClassEvidence::Unknown)
        .then(|| (proof.classes, proof.helpers.into_iter().collect()))
}

pub(super) fn original_object_preservation(
    model: &SemanticModel,
    call: &CallExpr,
    formals: &BTreeSet<String>,
) -> Option<Vec<FunctionRef>> {
    let root = call.syntax().ancestors().last()?;
    let body = call
        .syntax()
        .ancestors()
        .find_map(FunctionExpr::cast)?
        .body()?;
    let mut lookup = Lookup::new(model, &root, None);
    let mut env = Environment::default();
    // Keep the incoming object separately from formal rebindings. Assume it
    // can be a reference object, so any possible alias mutation loses evidence.
    let original = Value {
        proof: Proof::absent(),
        ..Value::default()
    };
    for formal in formals {
        env.values.insert(formal.clone(), original.clone());
    }
    env.retained.push(original);
    lookup.prefix(&body, call.syntax().text_range(), &mut env, 0);
    (env.retained.first()?.proof.classes == S3ClassEvidence::NoExplicitClass)
        .then(|| lookup.helpers.into_iter().collect())
}

#[derive(Clone, Default)]
struct Proof {
    classes: S3ClassEvidence,
    helpers: BTreeSet<FunctionRef>,
}

impl Proof {
    fn absent() -> Self {
        Self {
            classes: S3ClassEvidence::NoExplicitClass,
            ..Self::default()
        }
    }

    fn vector(names: Vec<String>) -> Self {
        if names.is_empty() {
            Self::absent()
        } else {
            Self {
                classes: S3ClassEvidence::Explicit(names),
                ..Self::default()
            }
        }
    }
}

#[derive(Clone, Default)]
struct Value {
    proof: Proof,
    vector: Option<Vec<String>>,
    function: Option<FunctionExpr>,
    returned: bool,
    by_value: bool,
}

impl Value {
    fn absent() -> Self {
        Self {
            proof: Proof::absent(),
            by_value: true,
            ..Self::default()
        }
    }

    fn vector(vector: Vec<String>) -> Self {
        Self {
            vector: Some(vector),
            ..Self::absent()
        }
    }
}

#[derive(Default)]
struct Environment {
    values: BTreeMap<String, Value>,
    // Evaluated objects survive rebinding, but reference mutations can affect them.
    retained: Vec<Value>,
    opaque: bool,
}

impl Environment {
    fn invalidate(&mut self) {
        self.values
            .values_mut()
            .for_each(|value| *value = Value::default());
        self.retained
            .iter_mut()
            .filter(|value| !value.by_value)
            .for_each(|value| *value = Value::default());
        self.opaque = true;
    }
}

struct Lookup<'a> {
    model: &'a SemanticModel,
    root: &'a SyntaxNode,
    project: Option<(&'a ClassIndex, &'a Path)>,
    visiting: BTreeSet<TextSize>,
    helpers: BTreeSet<FunctionRef>,
}

impl<'a> Lookup<'a> {
    fn new(
        model: &'a SemanticModel,
        root: &'a SyntaxNode,
        project: Option<(&'a ClassIndex, &'a Path)>,
    ) -> Self {
        Self {
            model,
            root,
            project,
            visiting: BTreeSet::new(),
            helpers: BTreeSet::new(),
        }
    }

    // Only helpers with confirmed provenance establish evidence. During the
    // range-free projection, retain their origins for confirmation in ClassIndex.
    fn helper(&self, call: &CallExpr, env: &Environment) -> Option<(String, FunctionRef)> {
        let reference = call_function_ref(self.model, self.root, call)?;
        let identity = match &reference {
            FunctionRef::Qualified { package, name } if package == "base" => name.clone(),
            FunctionRef::Bare { name, .. } if !env.opaque => name.clone(),
            _ => return None,
        };
        if !HELPERS.contains(&identity.as_str()) {
            return None;
        }
        if let Some((index, path)) = self.project
            && !index
                .callee_identity(path, &reference)
                .is_some_and(|origin| origin.package == "base" && origin.name == identity)
        {
            return None;
        }
        Some((identity, reference))
    }

    fn prefix(
        &mut self,
        container: &SyntaxElement,
        target: TextRange,
        env: &mut Environment,
        depth: usize,
    ) {
        if container.text_range() == target {
            return;
        }
        if depth >= 32 {
            env.invalidate();
            return;
        }
        for child in container
            .as_node()
            .into_iter()
            .flat_map(SyntaxNode::children_with_tokens)
        {
            if child.text_range().end() <= target.start() {
                if is_expression(&child) && self.eval(&child, env, depth + 1).returned {
                    env.invalidate();
                    return;
                }
            } else if child.text_range().contains_range(target) {
                // A branch/loop may have run earlier iterations or other arms.
                // Do not turn its source order into a runtime ordering claim.
                if matches!(
                    child.kind(),
                    SyntaxKind::IF_EXPR
                        | SyntaxKind::FOR_EXPR
                        | SyntaxKind::WHILE_EXPR
                        | SyntaxKind::REPEAT_EXPR
                ) {
                    self.invalidate_writes_except(&child, env, Some(target));
                } else {
                    if child.kind() == SyntaxKind::CALL_EXPR {
                        self.invalidate_writes_except(&child, env, Some(target));
                    }
                    self.prefix(&child, target, env, depth + 1);
                }
                break;
            }
        }
    }

    fn eval(&mut self, value: &SyntaxElement, env: &mut Environment, depth: usize) -> Value {
        if depth >= 32 {
            // The skipped subtree may write bindings or call a mutator. A
            // depth bound must lose facts rather than retain stale evidence.
            env.invalidate();
            return Value::default();
        }
        if let Some(token) = value.as_token() {
            return match token.kind() {
                SyntaxKind::STRING => crate::semantic::decode_string_literal(token.text())
                    .map(|name| Value::vector(vec![name]))
                    .unwrap_or_default(),
                SyntaxKind::INT | SyntaxKind::FLOAT | SyntaxKind::COMPLEX => Value::absent(),
                SyntaxKind::IDENT => match token.text() {
                    "NULL" => Value::vector(Vec::new()),
                    "TRUE" | "FALSE" | "NA" | "NA_integer_" | "NA_real_" | "NA_character_"
                    | "NA_complex_" | "Inf" | "NaN" => Value::absent(),
                    name => env
                        .values
                        .get(unbacktick(name))
                        .cloned()
                        .unwrap_or_default(),
                },
                _ => Value::default(),
            };
        }
        let node = value.as_node().unwrap();
        if let Some(paren) = ParenExpr::cast(node.clone()) {
            return paren
                .inner()
                .map(|inner| self.eval(&inner, env, depth + 1))
                .unwrap_or_default();
        }
        if let Some(block) = BlockExpr::cast(node.clone()) {
            let mut result = Value::absent();
            for statement in block.statements() {
                result = self.eval(&statement, env, depth + 1);
                if result.returned {
                    break;
                }
            }
            return result;
        }
        if let Some(function) = FunctionExpr::cast(node.clone()) {
            return Value {
                function: Some(function),
                ..Value::absent()
            };
        }
        if let Some(assignment) = AssignmentExpr::cast(node.clone()) {
            return self.assignment(&assignment, env, depth + 1);
        }
        if let Some(call) = CallExpr::cast(node.clone()) {
            return self.call(&call, env, depth + 1);
        }
        self.invalidate_writes(value, env);
        Value::default()
    }

    fn assignment(
        &mut self,
        assignment: &AssignmentExpr,
        env: &mut Environment,
        depth: usize,
    ) -> Value {
        if assignment
            .op_token()
            .is_some_and(|token| matches!(token.text(), "<<-" | "->>"))
        {
            env.invalidate();
            return Value::default();
        }
        let Some(rhs) = assignment.value_element() else {
            return Value::default();
        };
        if let Some(name) = assignment.target_name() {
            let result = self.eval(&rhs, env, depth + 1);
            env.values.insert(unbacktick(&name).into(), result.clone());
            return result;
        }
        if let Some(call) = assignment
            .target_element()
            .and_then(SyntaxElement::into_node)
            .and_then(CallExpr::cast)
            && let Some((name, helper)) = self.helper(&call, env)
            && matches!(name.as_str(), "class<-" | "oldClass<-")
            && let Some(target) = matched_args(&call, &["x"]).and_then(|args| argument(&args, "x"))
            && let Some(token) = target
                .as_token()
                .filter(|token| token.kind() == SyntaxKind::IDENT)
        {
            self.helpers.insert(helper.clone());
            let vector = self.eval(&rhs, env, depth + 1);
            let mut proof = class_proof(&name, &vector);
            proof.helpers.insert(helper);
            let by_value = env
                .values
                .get(unbacktick(token.text()))
                .is_some_and(|value| value.by_value);
            if let Some(data) = env.values.get(unbacktick(token.text())) {
                proof.helpers.extend(data.proof.helpers.iter().cloned());
            }
            invalidate_reference_aliases(env, by_value);
            env.values.insert(
                unbacktick(token.text()).into(),
                Value {
                    proof,
                    by_value,
                    ..Value::default()
                },
            );
            return vector;
        }
        self.invalidate_writes(&assignment.syntax().clone().into(), env);
        Value::default()
    }

    fn call(&mut self, call: &CallExpr, env: &mut Environment, depth: usize) -> Value {
        if let Some((name, helper)) = self.helper(call, env) {
            self.helpers.insert(helper.clone());
            self.invalidate_writes(&call.syntax().clone().into(), env);
            let mut result = self.helper_call(call, &name, env, depth + 1);
            result.proof.helpers.insert(helper);
            return result;
        }
        let function = call.base().and_then(|base| {
            if let Some(token) = base.as_token() {
                if let Some(value) = env.values.get(unbacktick(token.text())) {
                    return value.function.clone();
                }
                if env.opaque {
                    return None;
                }
                let id = read_binding(self.model, token)?;
                let binding = self.model.binding(id);
                let same_frame = self
                    .model
                    .innermost_scope_at(call.syntax().text_range().start())
                    == binding.scope;
                if same_frame && binding.available_from > call.syntax().text_range().start() {
                    return None;
                }
                return stable_value(self.model, self.root, id)?
                    .into_node()
                    .and_then(FunctionExpr::cast);
            }
            base.into_node().and_then(FunctionExpr::cast)
        });
        // Calls with unevaluated writes cannot be simulated as eager execution.
        self.invalidate_writes(&call.syntax().clone().into(), env);
        invalidate_reference_aliases(env, false);
        if let Some(function) = function {
            let mut result = self.function(&function, env, depth + 1);
            result.vector = None;
            return result;
        }
        let result = self.project_call(call, env);
        // Published constructors have fully analyzed bodies. Every other
        // unresolved call may replace even a by-value caller binding.
        if result.proof.classes == S3ClassEvidence::Unknown && !self.known_source(call) {
            env.invalidate();
        }
        result
    }

    fn helper_call(
        &mut self,
        call: &CallExpr,
        name: &str,
        env: &mut Environment,
        depth: usize,
    ) -> Value {
        match name {
            ".S3method" | "registerS3method" => {
                // Registration updates a dispatch table, not the caller's
                // object bindings. Argument expressions may still have effects.
                self.invalidate_writes(&call.syntax().clone().into(), env);
                Value::absent()
            }
            "structure" => {
                let Some(args) = matched_args(call, &[".Data", "..."]) else {
                    return Value::default();
                };
                let Some(data) = argument(&args, ".Data") else {
                    return Value::default();
                };
                let data = self.eval(&data, env, depth + 1);
                let Some(supplied) = call.arg_list() else {
                    return Value::default();
                };
                let class = supplied
                    .args()
                    .filter(|arg| arg.name().as_deref() == Some("class"))
                    .last();
                if let Some(class) = class {
                    let Some(value) = class.value() else {
                        return Value::default();
                    };
                    let vector = self.eval(&value, env, depth + 1);
                    invalidate_reference_aliases(env, data.by_value);
                    let mut proof = class_proof(name, &vector);
                    proof.helpers.extend(data.proof.helpers);
                    Value {
                        proof,
                        by_value: data.by_value,
                        ..Value::default()
                    }
                } else {
                    data
                }
            }
            "class<-" | "oldClass<-" => {
                let Some(args) = matched_args(call, &["x", "value"]) else {
                    return Value::default();
                };
                if !primitive_argument_order(call, &["x", "value"]) {
                    return Value::default();
                }
                let Some(data) = argument(&args, "x") else {
                    return Value::default();
                };
                let data = self.eval(&data, env, depth + 1);
                let Some(value) = argument(&args, "value") else {
                    return Value::default();
                };
                let vector = self.eval(&value, env, depth + 1);
                invalidate_reference_aliases(env, data.by_value);
                let mut proof = class_proof(name, &vector);
                proof.helpers.extend(data.proof.helpers);
                Value {
                    proof,
                    by_value: data.by_value,
                    ..Value::default()
                }
            }
            "return" => {
                let mut value = match call.arg_list() {
                    Some(args) if args.args().next().is_none() => Value::absent(),
                    Some(args) if args.args().count() == 1 => args
                        .args()
                        .next()
                        .and_then(|arg| arg.value())
                        .map(|value| self.eval(&value, env, depth + 1))
                        .unwrap_or_default(),
                    _ => Value::default(),
                };
                value.returned = true;
                value
            }
            "class" | "oldClass" | "unclass" => {
                let Some(args) = matched_args(call, &["x"]) else {
                    return Value::default();
                };
                if !primitive_argument_order(call, &["x"]) {
                    return Value::default();
                }
                let value = argument(&args, "x")
                    .map(|value| self.eval(&value, env, depth + 1))
                    .unwrap_or_default();
                let vector = match (&value.proof.classes, name) {
                    (S3ClassEvidence::Explicit(names), _) => Some(names.clone()),
                    (S3ClassEvidence::NoExplicitClass, "oldClass") => Some(Vec::new()),
                    _ => None,
                };
                let mut proof = Proof::absent();
                proof.helpers = value.proof.helpers;
                Value {
                    proof,
                    vector: if name == "unclass" { None } else { vector },
                    ..Value::absent()
                }
            }
            "c" => {
                if matched_args(call, &["..."]).is_none() {
                    return Value::default();
                }
                let mut result = Value::vector(Vec::new());
                let Some(supplied) = call.arg_list() else {
                    return Value::default();
                };
                for arg in supplied.args() {
                    if matches!(arg.name().as_deref(), Some("recursive" | "use.names")) {
                        return Value::default();
                    }
                    let Some(value) = arg.value() else {
                        return Value::default();
                    };
                    let value = self.eval(&value, env, depth + 1);
                    let Some(vector) = value.vector else {
                        return Value::default();
                    };
                    result.vector.as_mut().unwrap().extend(vector);
                    result.proof.helpers.extend(value.proof.helpers);
                }
                result
            }
            "character" => {
                if matched_args(call, &["length"]).is_some_and(|args| {
                    argument(&args, "length").is_none_or(|value| {
                        value
                            .as_token()
                            .is_some_and(|token| matches!(token.text(), "0" | "0L"))
                    })
                }) {
                    Value::vector(Vec::new())
                } else {
                    Value::default()
                }
            }
            "list" => {
                let Some(args) = call.arg_list() else {
                    return Value::default();
                };
                for value in args.args().filter_map(|arg| arg.value()) {
                    self.eval(&value, env, depth + 1);
                }
                Value::absent()
            }
            _ => Value::default(),
        }
    }

    fn function(
        &mut self,
        function: &FunctionExpr,
        caller: &mut Environment,
        depth: usize,
    ) -> Value {
        if depth >= 32 || !self.visiting.insert(function.syntax().text_range().start()) {
            caller.invalidate();
            return Value::default();
        }
        // A bounded constructor is a straight-line body. Conditional returns,
        // loops, lazy side effects, and dynamic environments remain unknown.
        // Entering another frame cannot restore trust in bindings that an
        // earlier caller left opaque.
        let mut env = Environment {
            opaque: caller.opaque,
            ..Environment::default()
        };
        let result = if self.opaque_function(function) {
            env.invalidate();
            Value::default()
        } else {
            function
                .body()
                .map(|body| self.eval(&body, &mut env, depth + 1))
                .unwrap_or_default()
        };
        self.visiting
            .remove(&function.syntax().text_range().start());
        if env.opaque {
            // Effects from a nested call must reach every caller, including
            // wrappers whose own syntax contains no nonlocal assignment.
            caller.invalidate();
            return Value::default();
        }
        Value {
            returned: false,
            ..result
        }
    }

    fn project_call(&self, call: &CallExpr, env: &Environment) -> Value {
        let Some((index, path)) = self.project else {
            return Value::default();
        };
        let Some(base) = call.base() else {
            return Value::default();
        };
        let (name, files) = if let Some(token) = base.as_token() {
            if env.opaque || read_binding(self.model, token).is_some() {
                return Value::default();
            }
            let name = unbacktick(token.text());
            let deferred = call
                .syntax()
                .ancestors()
                .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR);
            let mut files = index.definition_files(path, name, deferred);
            if files.is_empty() {
                files = index.implementation_files(path, name, None);
            }
            (name.to_string(), files)
        } else if let Some(namespace) = base
            .into_node()
            .and_then(BinaryExpr::cast)
            .and_then(|binary| binary.namespace_access())
        {
            (
                namespace.name.to_string(),
                index.implementation_files(path, &namespace.name, Some(&namespace.package)),
            )
        } else {
            return Value::default();
        };
        if files.len() != 1 {
            return Value::default();
        }
        let mut constructors = files
            .iter()
            .filter_map(|file| index.file(file))
            .flat_map(|file| {
                file.facts
                    .s3
                    .constructors
                    .iter()
                    .filter(|constructor| constructor.binding == name)
                    .map(move |constructor| (file, constructor))
            });
        let Some((file, constructor)) = constructors.next() else {
            return Value::default();
        };
        if constructors.next().is_some()
            || !index.class_file_complete(path)
            || !index.class_file_complete(&file.path)
            || !constructor.helpers.iter().all(|helper| {
                index
                    .callee_identity(&file.path, helper)
                    .is_some_and(|origin| origin.package == "base")
            })
        {
            return Value::default();
        }
        Value {
            proof: Proof {
                classes: constructor.classes.clone(),
                helpers: BTreeSet::new(),
            },
            by_value: constructor.by_value,
            ..Value::default()
        }
    }

    fn invalidate_writes(&self, value: &SyntaxElement, env: &mut Environment) {
        self.invalidate_writes_except(value, env, None);
    }

    fn invalidate_writes_except(
        &self,
        value: &SyntaxElement,
        env: &mut Environment,
        exclude: Option<TextRange>,
    ) {
        let Some(node) = value.as_node() else {
            return;
        };
        for node in node.descendants().filter(|node| {
            !exclude.is_some_and(|range| {
                range.contains_range(node.text_range()) || node.text_range().contains_range(range)
            }) && !node
                .ancestors()
                .take_while(|parent| parent != value.as_node().unwrap())
                .any(|parent| parent.kind() == SyntaxKind::FUNCTION_EXPR)
        }) {
            if let Some(assignment) = AssignmentExpr::cast(node.clone()) {
                if assignment
                    .op_token()
                    .is_some_and(|token| matches!(token.text(), "<<-" | "->>"))
                {
                    env.invalidate();
                }
                if let Some(target) = assignment.target_element()
                    && let Some(name) = target_name(&target)
                {
                    if env.values.get(&name).is_some_and(|value| !value.by_value) {
                        invalidate_reference_aliases(env, false);
                    }
                    env.values.insert(name, Value::default());
                }
            }
            if let Some(call) = CallExpr::cast(node.clone())
                && node.text_range() != value.text_range()
            {
                match self.helper(&call, env) {
                    Some((name, _))
                        if matches!(
                            name.as_str(),
                            "class"
                                | "oldClass"
                                | "unclass"
                                | "c"
                                | "list"
                                | "character"
                                | "return"
                        ) => {}
                    Some(_) => invalidate_reference_aliases(env, false),
                    None => env.invalidate(),
                }
            }
            if let Some(call) = CallExpr::cast(node)
                && call
                    .callee_name()
                    .is_some_and(|name| crate::semantic::calls::changes_bindings(&name))
            {
                if self.known_source(&call) {
                    env.values
                        .values_mut()
                        .for_each(|value| *value = Value::default());
                } else {
                    env.invalidate();
                }
            }
        }
    }

    fn known_source(&self, call: &CallExpr) -> bool {
        self.project.is_some_and(|(index, path)| {
            index.class_file_complete(path)
                && call
                    .syntax()
                    .parent()
                    .is_some_and(|node| node.kind() == SyntaxKind::ROOT)
                && crate::project::source::top_level_source_edge_key(call.syntax(), path.parent())
                    .is_some_and(|edge| {
                        matches!(edge.target, crate::project::source::SourceTarget::Path(_))
                            && !edge.local
                    })
        })
    }

    fn opaque_function(&self, function: &FunctionExpr) -> bool {
        function
            .syntax()
            .descendants()
            .skip(1)
            .filter(|node| {
                !node
                    .ancestors()
                    .skip(1)
                    .take_while(|node| node != function.syntax())
                    .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
            })
            .any(|node| {
                matches!(
                    node.kind(),
                    SyntaxKind::IF_EXPR
                        | SyntaxKind::FOR_EXPR
                        | SyntaxKind::WHILE_EXPR
                        | SyntaxKind::REPEAT_EXPR
                ) || CallExpr::cast(node.clone()).is_some_and(|call| {
                    let opaque = |name: &str| {
                        crate::semantic::calls::changes_bindings(name)
                            || matches!(name, "UseMethod" | "NextMethod" | "on.exit")
                            || (name == "return"
                                && node
                                    .ancestors()
                                    .skip(1)
                                    .take_while(|parent| parent != function.syntax())
                                    .any(|parent| {
                                        !matches!(
                                            parent.kind(),
                                            SyntaxKind::BLOCK_EXPR | SyntaxKind::PAREN_EXPR
                                        )
                                    }))
                    };
                    // Even ambiguous origins can rule out an exact summary.
                    // Never screen control flow using an alias's spelling.
                    match call_function_ref(self.model, self.root, &call) {
                        Some(FunctionRef::Bare { name, .. }) => opaque(&name),
                        Some(FunctionRef::Qualified { package, name }) => {
                            package == "base" && opaque(&name)
                        }
                        Some(FunctionRef::Unresolved(candidates)) => candidates
                            .iter()
                            .any(|origin| origin.package == "base" && opaque(&origin.name)),
                        None => false,
                    }
                }) || AssignmentExpr::cast(node)
                    .and_then(|assignment| assignment.op_token())
                    .is_some_and(|token| matches!(token.text(), "<<-" | "->>"))
            })
    }
}

fn is_expression(element: &SyntaxElement) -> bool {
    Expr::cast(element.clone()).is_some()
}

fn target_name(value: &SyntaxElement) -> Option<String> {
    if let Some(token) = value
        .as_token()
        .filter(|token| token.kind() == SyntaxKind::IDENT)
    {
        return Some(unbacktick(token.text()).into());
    }
    if let Some(call) = value.as_node().cloned().and_then(CallExpr::cast) {
        return target_name(&call.arg_list()?.args().next()?.value()?);
    }
    value.as_node()?.children_with_tokens().find_map(|element| {
        is_expression(&element)
            .then(|| target_name(&element))
            .flatten()
    })
}

fn primitive_argument_order(call: &CallExpr, formals: &[&str]) -> bool {
    call.arg_list().is_some_and(|args| {
        let supplied: Vec<_> = args.args().collect();
        supplied.len() == formals.len()
            && supplied.iter().zip(formals).all(|(arg, formal)| {
                arg.value().is_some() && arg.name().is_none_or(|name| name == *formal)
            })
    })
}

fn class_proof(setter: &str, vector: &Value) -> Proof {
    let Some(names) = &vector.vector else {
        return Proof::default();
    };
    let mut proof = Proof::vector(names.clone());
    // class<- coerces a single basic class and removes the explicit attribute.
    // structure and oldClass<- set the attribute directly instead.
    if setter == "class<-"
        && names.len() == 1
        && matches!(
            names[0].as_str(),
            "logical"
                | "integer"
                | "double"
                | "numeric"
                | "complex"
                | "character"
                | "raw"
                | "list"
                | "expression"
                | "matrix"
                | "array"
                | "name"
                | "environment"
                | "externalptr"
                | "weakref"
                | "char"
        )
    {
        proof.classes = S3ClassEvidence::NoExplicitClass;
    }
    if setter == "class<-" && names == &["matrix", "array"] {
        proof.classes = S3ClassEvidence::NoExplicitClass;
    }
    proof.helpers.extend(vector.proof.helpers.iter().cloned());
    proof
}

fn invalidate_reference_aliases(env: &mut Environment, by_value: bool) {
    if !by_value {
        for value in env
            .values
            .values_mut()
            .chain(&mut env.retained)
            .filter(|value| !value.by_value)
        {
            *value = Value::default();
        }
    }
}
