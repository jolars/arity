//! Replay source effects over abstract environments. Function analysis forks
//! state, and branch joins preserve possible bindings and destinations.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::{
    BindingKey, Environment, Event, ReadKey, SourceEdgeKey, SourceProgram, SourceRead, SourceReads,
};
use crate::incremental::normalize_path;
use crate::project::FileFacts;
use crate::project::source::SourceTarget;
use crate::semantic::symbols::unbacktick;

#[derive(Debug, Default)]
pub(crate) struct SourceEnvironments {
    pub reads: HashMap<PathBuf, SourceReads>,
    pub used: HashMap<PathBuf, BTreeSet<String>>,
    pub member_uses: HashMap<PathBuf, BTreeSet<String>>,
    pub readers: HashMap<PathBuf, BTreeMap<String, BTreeSet<PathBuf>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Value {
    definitions: BTreeSet<Definition>,
    environment: Environments,
    uncertain: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Environments {
    frames: BTreeSet<usize>,
    /// An alternative value has no statically known destination.
    unknown: bool,
}

impl Environments {
    fn known(frame: usize) -> Self {
        Self {
            frames: BTreeSet::from([frame]),
            unknown: false,
        }
    }

    fn unknown() -> Self {
        Self {
            unknown: true,
            ..Self::default()
        }
    }

    fn extend(&mut self, other: &Self) {
        self.frames.extend(&other.frames);
        self.unknown |= other.unknown;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Definition {
    path: PathBuf,
    name: String,
    key: Option<BindingKey>,
    exported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Frame {
    owner: PathBuf,
    /// Helper environment members must not mask an introduced bare call.
    execution: bool,
    parent: Option<usize>,
    values: BTreeMap<String, Value>,
    opaque: bool,
}

#[derive(Clone)]
struct Function {
    path: PathBuf,
    id: usize,
    program: SourceProgram,
    enclosing: usize,
    directory: Option<PathBuf>,
}

#[derive(Clone)]
struct Deferred {
    path: PathBuf,
    program: SourceProgram,
    enclosing: usize,
}

#[derive(Clone, Default)]
struct State {
    frames: Vec<Frame>,
    functions: Vec<Function>,
    deferred: Vec<Deferred>,
}

impl State {
    fn frame(&mut self, parent: Option<usize>, owner: &Path, execution: bool) -> usize {
        let id = self.frames.len();
        self.frames.push(Frame {
            parent,
            owner: owner.to_path_buf(),
            execution,
            ..Frame::default()
        });
        id
    }

    fn lookup(&self, mut frame: usize, name: &str) -> Result<Option<Cow<'_, Value>>, ()> {
        let mut found: Option<Cow<'_, Value>> = None;
        loop {
            let current = &self.frames[frame];
            if let Some(value) = current.values.get(unbacktick(name)) {
                match &mut found {
                    None => found = Some(Cow::Borrowed(value)),
                    Some(previous) => {
                        let previous = previous.to_mut();
                        previous
                            .definitions
                            .extend(value.definitions.iter().cloned());
                        previous.environment.extend(&value.environment);
                    }
                }
                if !value.uncertain {
                    if let Some(found) = &mut found
                        && found.uncertain
                    {
                        found.to_mut().uncertain = false;
                    }
                    return Ok(found);
                }
            }
            if current.opaque {
                return match found {
                    Some(mut value) => {
                        value.to_mut().uncertain = true;
                        Ok(Some(value))
                    }
                    None => Err(()),
                };
            }
            match current.parent {
                Some(parent) => frame = parent,
                None => return Ok(found),
            }
        }
    }

    fn lookup_member(&self, frame: usize, name: &str) -> Result<Option<Cow<'_, Value>>, ()> {
        let frame = &self.frames[frame];
        match frame.values.get(unbacktick(name)) {
            Some(value) => Ok(Some(Cow::Borrowed(value))),
            None if frame.opaque => Err(()),
            None => Ok(None),
        }
    }

    fn environment(&mut self, expression: &Environment, current: usize) -> Environments {
        match expression {
            Environment::Global => Environments::known(0),
            Environment::Current => Environments::known(current),
            Environment::Named(name) => {
                let fallback = match name.as_str() {
                    "F" => Some(0),
                    "T" => Some(current),
                    _ => None,
                };
                match self.lookup(current, name) {
                    Ok(Some(value)) => {
                        let mut environment = value.environment.clone();
                        if value.uncertain {
                            if let Some(fallback) = fallback {
                                environment.frames.insert(fallback);
                            } else {
                                environment.unknown = true;
                            }
                        }
                        environment
                    }
                    Ok(None) => fallback.map_or_else(Environments::unknown, Environments::known),
                    Err(()) => {
                        let mut environment = Environments::unknown();
                        environment.frames.extend(fallback);
                        environment
                    }
                }
            }
            Environment::Fresh(parent) => {
                let parents = self.environment(parent, current);
                let owner = self.frames[current].owner.clone();
                let mut environments = Environments::default();
                for parent in parents.frames {
                    environments
                        .frames
                        .insert(self.frame(Some(parent), &owner, false));
                }
                if parents.unknown || environments.frames.is_empty() {
                    let frame = self.frame(None, &owner, false);
                    self.frames[frame].opaque = !self.is_base(parent, current);
                    environments.frames.insert(frame);
                }
                environments
            }
            Environment::Base | Environment::Unknown => Environments::unknown(),
            Environment::Builtin(name, value) => match self.lookup(current, name) {
                Ok(None) => self.environment(value, current),
                Ok(Some(binding)) if !binding.uncertain => Environments::unknown(),
                _ => {
                    let mut environment = self.environment(value, current);
                    environment.unknown = true;
                    environment
                }
            },
        }
    }

    fn is_base(&self, expression: &Environment, current: usize) -> bool {
        match expression {
            Environment::Base => true,
            Environment::Builtin(name, value) => {
                self.lookup(current, name)
                    .is_ok_and(|value| value.is_none())
                    && self.is_base(value, current)
            }
            _ => false,
        }
    }

    fn join(&mut self, left: State, mut right: State) {
        let original_frames = self.frames.len();
        let original_functions = self.functions.len();
        let original_deferred = self.deferred.len();
        // The shared prefix retains its identities. Each branch's new frames
        // occupy a separate suffix so aliases and closure parents remain valid.
        let shift = left.frames.len() - original_frames;
        let remap = |frame: usize| {
            if frame < original_frames {
                frame
            } else {
                frame + shift
            }
        };
        for frame in &mut right.frames {
            frame.parent = frame.parent.map(remap);
            for value in frame.values.values_mut() {
                value.environment.frames = value
                    .environment
                    .frames
                    .iter()
                    .copied()
                    .map(remap)
                    .collect();
            }
        }
        for function in &mut right.functions {
            function.enclosing = remap(function.enclosing);
        }
        for deferred in &mut right.deferred {
            deferred.enclosing = remap(deferred.enclosing);
        }
        for (index, frame) in self.frames.iter_mut().enumerate() {
            let a = &left.frames[index];
            let b = &right.frames[index];
            frame.values.clear();
            for name in a.values.keys().chain(b.values.keys()) {
                let left = a.values.get(name);
                let right = b.values.get(name);
                let mut environment = Environments::default();
                for value in left.into_iter().chain(right) {
                    environment.extend(&value.environment);
                }
                let definitions = left
                    .into_iter()
                    .chain(right)
                    .flat_map(|value| value.definitions.iter().cloned())
                    .collect();
                let uncertain = left.is_none_or(|value| value.uncertain)
                    || right.is_none_or(|value| value.uncertain);
                frame.values.insert(
                    name.clone(),
                    Value {
                        environment,
                        definitions,
                        uncertain,
                    },
                );
            }
            frame.opaque = a.opaque || b.opaque;
        }
        self.frames
            .extend(left.frames.into_iter().skip(original_frames));
        self.frames
            .extend(right.frames.into_iter().skip(original_frames));
        // Bodies declared on either path are worth checking, but their effects
        // are confined to the separate function-analysis fork.
        for function in left
            .functions
            .into_iter()
            .skip(original_functions)
            .chain(right.functions.into_iter().skip(original_functions))
        {
            if function.enclosing < self.frames.len() {
                self.functions.push(function);
            }
        }
        for read in left
            .deferred
            .into_iter()
            .skip(original_deferred)
            .chain(right.deferred.into_iter().skip(original_deferred))
        {
            if read.enclosing < self.frames.len() {
                self.deferred.push(read);
            }
        }
    }
}

struct Replay<'a> {
    files: HashMap<PathBuf, &'a FileFacts>,
    output: SourceEnvironments,
    source_stack: HashSet<(PathBuf, Option<PathBuf>)>,
    function_stack: HashSet<(PathBuf, usize)>,
}

enum ReadKind<'a> {
    Name(Option<&'a ReadKey>),
    Member,
    Runtime,
}

impl SourceEnvironments {
    pub fn build(
        files: &[FileFacts],
        initial: &HashMap<PathBuf, BTreeMap<String, PathBuf>>,
    ) -> Self {
        let mut replay = Replay {
            files: files
                .iter()
                .map(|file| (normalize_path(&file.path), file))
                .collect(),
            output: Self::default(),
            source_stack: HashSet::new(),
            function_stack: HashSet::new(),
        };
        for file in files {
            let Some(program) = file.source_program.as_deref() else {
                continue;
            };
            if !program.has_sources() && file.package_root.is_none() {
                continue;
            }
            let mut state = State::default();
            let global = state.frame(None, &file.path, true);
            let current = if file.package_root.is_some() {
                state.frame(Some(global), &file.path, true)
            } else {
                global
            };
            if let Some(names) = initial.get(&file.path) {
                for (name, definer) in names {
                    state.frames[current].values.insert(
                        unbacktick(name).to_string(),
                        Value {
                            definitions: BTreeSet::from([Definition {
                                path: definer.clone(),
                                name: name.clone(),
                                key: None,
                                exported: true,
                            }]),
                            environment: Environments::unknown(),
                            uncertain: false,
                        },
                    );
                }
            }
            replay.program(
                &mut state,
                program,
                &file.path,
                current,
                file.execution_directory.as_deref(),
            );
            replay.functions(&state);
        }
        replay.output
    }
}

impl Replay<'_> {
    fn read(&mut self, state: &State, current: usize, path: &Path, name: &str, kind: ReadKind<'_>) {
        let value = match kind {
            ReadKind::Member => state.lookup_member(current, name),
            _ => state.lookup(current, name),
        };
        let status = match value {
            Ok(Some(value)) => {
                for definition in &value.definitions {
                    if matches!(kind, ReadKind::Member | ReadKind::Runtime) && definition.exported {
                        self.output
                            .member_uses
                            .entry(definition.path.clone())
                            .or_default()
                            .insert(definition.name.clone());
                    }
                    if definition.path != path && definition.exported {
                        self.output
                            .used
                            .entry(definition.path.clone())
                            .or_default()
                            .insert(definition.name.clone());
                        self.output
                            .readers
                            .entry(definition.path.clone())
                            .or_default()
                            .entry(definition.name.clone())
                            .or_default()
                            .insert(path.to_path_buf());
                    }
                    if let Some(key) = &definition.key {
                        self.output
                            .reads
                            .entry(definition.path.clone())
                            .or_default()
                            .used_bindings
                            .insert(key.clone());
                    }
                }
                match value.definitions.len() {
                    _ if value.uncertain => SourceRead::Unknown,
                    0 => SourceRead::Bound(path.to_path_buf()),
                    1 => SourceRead::Bound(
                        value
                            .definitions
                            .first()
                            .expect("one definition")
                            .path
                            .clone(),
                    ),
                    _ => SourceRead::Unknown,
                }
            }
            Ok(None) => SourceRead::Unbound,
            Err(()) => SourceRead::Unknown,
        };
        if let ReadKind::Name(Some(key)) = kind {
            self.output
                .reads
                .entry(path.to_path_buf())
                .or_default()
                .reads
                .entry(key.clone())
                .and_modify(|previous| {
                    if *previous != status {
                        *previous = SourceRead::Unknown;
                    }
                })
                .or_insert(status);
        }
    }

    fn program(
        &mut self,
        state: &mut State,
        program: &SourceProgram,
        path: &Path,
        current: usize,
        directory: Option<&Path>,
    ) {
        for event in &program.0 {
            match event {
                Event::Read { name, key } => {
                    self.read(state, current, path, name, ReadKind::Name(key.as_ref()))
                }
                Event::Member { receiver, name } => {
                    for environment in state.environment(receiver, current).frames {
                        self.read(state, environment, path, name, ReadKind::Member);
                    }
                }
                Event::UseOnly { name, gate } => {
                    if gate.as_ref().is_none_or(|gate| {
                        state
                            .lookup(current, gate)
                            .is_ok_and(|value| value.is_none())
                    }) {
                        self.read(state, current, path, name, ReadKind::Runtime);
                    }
                }
                Event::Define {
                    name,
                    value,
                    exported,
                    key,
                } => {
                    let frame = &state.frames[current];
                    if frame.execution {
                        self.output
                            .reads
                            .entry(frame.owner.clone())
                            .or_default()
                            .introduced_names
                            .insert(unbacktick(name).to_string());
                    }
                    let environment = state.environment(value, current);
                    state.frames[current].values.insert(
                        unbacktick(name).to_string(),
                        Value {
                            definitions: BTreeSet::from([Definition {
                                path: path.to_path_buf(),
                                name: name.clone(),
                                key: key.clone(),
                                exported: *exported,
                            }]),
                            environment,
                            uncertain: false,
                        },
                    );
                }
                Event::Function { id, program } => state.functions.push(Function {
                    path: path.to_path_buf(),
                    id: *id,
                    program: program.clone(),
                    enclosing: current,
                    directory: directory.map(Path::to_path_buf),
                }),
                Event::Deferred(program) => state.deferred.push(Deferred {
                    path: path.to_path_buf(),
                    program: program.clone(),
                    enclosing: current,
                }),
                Event::Branch(left, right) => {
                    let mut a = state.clone();
                    let mut b = state.clone();
                    self.program(&mut a, left, path, current, directory);
                    self.program(&mut b, right, path, current, directory);
                    self.functions(&a);
                    self.functions(&b);
                    state.join(a, b);
                }
                Event::Source {
                    edge,
                    destination,
                    bare,
                } => {
                    let uncertain_loader = if let Some(name) = bare {
                        match state.lookup(current, name) {
                            Ok(Some(value)) if !value.uncertain => continue,
                            Ok(None) => false,
                            _ => true,
                        }
                    } else {
                        false
                    };
                    let destinations = state.environment(destination, current);
                    let optional = uncertain_loader || destinations.unknown;
                    if let Some(&destination) = destinations.frames.first()
                        && destinations.frames.len() == 1
                        && !optional
                    {
                        self.source(state, edge, destination, directory);
                        continue;
                    }
                    let original = state.clone();
                    let mut joined = optional.then(|| original.clone());
                    for destination in destinations.frames {
                        let mut alternative = original.clone();
                        self.source(&mut alternative, edge, destination, directory);
                        joined = Some(match joined {
                            None => alternative,
                            Some(previous) => {
                                let mut merged = original.clone();
                                merged.join(previous, alternative);
                                merged
                            }
                        });
                    }
                    if let Some(joined) = joined {
                        *state = joined;
                    }
                }
            }
        }
    }

    fn source(
        &mut self,
        state: &mut State,
        edge: &SourceEdgeKey,
        destination: usize,
        directory: Option<&Path>,
    ) {
        let resolved = edge.resolve(directory);
        let SourceTarget::Path(target) = resolved.target else {
            self.opaque(state, destination);
            return;
        };
        let identity = (normalize_path(&target), resolved.directory.clone());
        let file = self.files.get(&identity.0).copied();
        let program = file.and_then(|file| file.source_program.as_deref());
        if let (Some(file), Some(program)) = (file, program)
            && self.source_stack.insert(identity.clone())
        {
            self.program(
                state,
                program,
                &file.path,
                destination,
                resolved.directory.as_deref(),
            );
            self.source_stack.remove(&identity);
        } else {
            self.opaque(state, destination);
        }
    }

    fn opaque(&mut self, state: &mut State, destination: usize) {
        let frame = &mut state.frames[destination];
        frame.opaque = true;
        if frame.execution {
            self.output
                .reads
                .entry(frame.owner.clone())
                .or_default()
                .introductions_incomplete = true;
        }
    }

    fn functions(&mut self, state: &State) {
        for deferred in &state.deferred {
            let mut local = state.clone();
            local.deferred.clear();
            self.program(
                &mut local,
                &deferred.program,
                &deferred.path,
                deferred.enclosing,
                None,
            );
        }
        for function in &state.functions {
            let identity = (function.path.clone(), function.id);
            if !self.function_stack.insert(identity.clone()) {
                continue;
            }
            let mut local = state.clone();
            local.functions.clear();
            local.deferred.clear();
            let current = local.frame(Some(function.enclosing), &function.path, true);
            self.program(
                &mut local,
                &function.program,
                &function.path,
                current,
                function.directory.as_deref(),
            );
            self.functions(&local);
            self.function_stack.remove(&identity);
        }
    }
}
