//! Working directories for statically known script executions. Discovery runs
//! in the write phase; graph traversal below is pure and keeps the caller's
//! directory until `source(..., chdir = TRUE)` temporarily changes it.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::incremental::normalize_path;

use super::scope::is_package_root;
use super::source::{SourceEdgeKey, SourceTarget};

pub(crate) fn discover_execution_root(path: &Path) -> Option<PathBuf> {
    let path = normalize_path(path);
    let parent = path.parent().unwrap_or(&path);
    let mut git = None;
    let mut config = None;
    for directory in parent.ancestors() {
        if is_package_root(directory) {
            return Some(directory.to_path_buf());
        }
        if git.is_none() && directory.join(".git").exists() {
            git = Some(directory.to_path_buf());
        }
        if config.is_none() && directory.join(crate::config::CONFIG_FILE_NAME).is_file() {
            config = Some(directory.to_path_buf());
        }
    }
    git.or(config)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SourceContext {
    pub path: PathBuf,
    pub directory: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedSource {
    pub target: SourceTarget,
    pub directory: Option<PathBuf>,
    pub local: bool,
}

impl SourceEdgeKey {
    pub(crate) fn resolve(&self, directory: Option<&Path>) -> ResolvedSource {
        let target = match (&self.target, &self.relative_path, directory) {
            (SourceTarget::Path(_), Some(relative), Some(directory)) => {
                SourceTarget::Path(normalize_path(&directory.join(relative)))
            }
            _ => self.target.clone(),
        };
        let directory = if self.chdir {
            match &target {
                SourceTarget::Path(path) => path.parent().map(Path::to_path_buf),
                SourceTarget::Dynamic => None,
            }
        } else {
            directory.map(Path::to_path_buf)
        };
        ResolvedSource {
            target,
            directory,
            local: self.local,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct SourceClosure {
    pub paths: HashSet<PathBuf>,
    pub incomplete: bool,
}

/// A file can execute under several directories (for example, two callers
/// with different `chdir` settings). Keep those executions distinct during
/// traversal; only visibility and use tracking take their union. Rename can
/// require the per-context binding results to agree.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct SourceGraph {
    /// Normalize for matching while preserving the spelling used by the rest
    /// of the project graph and diagnostics (including relative CLI paths).
    paths: HashMap<PathBuf, PathBuf>,
    entry_directories: BTreeMap<PathBuf, Option<PathBuf>>,
    directories: BTreeMap<PathBuf, BTreeSet<Option<PathBuf>>>,
    edges: BTreeMap<SourceContext, Vec<ResolvedSource>>,
}

impl SourceGraph {
    pub fn build<'a>(
        files: impl IntoIterator<Item = (&'a Path, Option<&'a Path>, &'a [SourceEdgeKey])>,
    ) -> Self {
        let mut graph = Self::default();
        let mut by_path = HashMap::new();
        let mut pending = Vec::new();
        for (path, directory, edges) in files {
            let directory = directory.map(normalize_path);
            graph.paths.insert(normalize_path(path), path.to_path_buf());
            graph
                .entry_directories
                .insert(path.to_path_buf(), directory.clone());
            by_path.insert(path, edges);
            pending.push(SourceContext {
                path: path.to_path_buf(),
                directory,
            });
        }
        while let Some(context) = pending.pop() {
            if graph.edges.contains_key(&context) {
                continue;
            }
            let Some(edges) = by_path.get(context.path.as_path()) else {
                continue;
            };
            graph
                .directories
                .entry(context.path.clone())
                .or_default()
                .insert(context.directory.clone());
            let edges: Vec<_> = edges
                .iter()
                .map(|edge| graph.resolve_edge(edge, &context))
                .collect();
            for edge in &edges {
                if let SourceTarget::Path(path) = &edge.target {
                    pending.push(SourceContext {
                        path: path.clone(),
                        directory: edge.directory.clone(),
                    });
                }
            }
            graph.edges.insert(context, edges);
        }
        graph
    }

    pub fn entry_context(&self, path: &Path) -> SourceContext {
        SourceContext {
            path: path.to_path_buf(),
            directory: self.entry_directories.get(path).cloned().flatten(),
        }
    }

    pub fn resolve_edge(&self, edge: &SourceEdgeKey, context: &SourceContext) -> ResolvedSource {
        let mut edge = edge.resolve(context.directory.as_deref());
        if let SourceTarget::Path(path) = &mut edge.target
            && let Some(member) = self.paths.get(&normalize_path(path))
        {
            *path = member.clone();
        }
        edge
    }

    pub fn directories(&self, path: &Path) -> impl Iterator<Item = Option<&Path>> {
        let normalized = normalize_path(path);
        let path = self.paths.get(&normalized).map_or(path, PathBuf::as_path);
        self.directories
            .get(path)
            .into_iter()
            .flatten()
            .map(Option::as_deref)
    }

    /// Include `start` itself, for folding a source call's exports into its
    /// caller. Cycles are guarded by file *and* working directory.
    pub fn closure(&self, start: SourceContext) -> SourceClosure {
        let mut closure = SourceClosure::default();
        let mut visited = HashSet::new();
        let mut pending = vec![start];
        while let Some(mut context) = pending.pop() {
            if let Some(path) = self.paths.get(&normalize_path(&context.path)) {
                context.path = path.clone();
            }
            if !visited.insert(context.clone()) {
                continue;
            }
            let Some(edges) = self.edges.get(&context) else {
                closure.incomplete = true;
                continue;
            };
            closure.paths.insert(context.path);
            for edge in edges {
                match &edge.target {
                    _ if edge.local => {}
                    SourceTarget::Dynamic => closure.incomplete = true,
                    SourceTarget::Path(path) => pending.push(SourceContext {
                        path: path.clone(),
                        directory: edge.directory.clone(),
                    }),
                }
            }
        }
        closure
    }

    pub fn sourced_from(&self, path: &Path) -> SourceClosure {
        let mut result = SourceClosure::default();
        for directory in self.directories(path) {
            let closure = self.closure(SourceContext {
                path: path.to_path_buf(),
                directory: directory.map(Path::to_path_buf),
            });
            result.paths.extend(closure.paths);
            result.incomplete |= closure.incomplete;
        }
        result.paths.remove(path);
        result
    }

    pub fn direct_edges(&self) -> impl Iterator<Item = (&Path, &ResolvedSource)> {
        self.edges.iter().flat_map(|(context, edges)| {
            edges.iter().map(move |edge| (context.path.as_path(), edge))
        })
    }
}
