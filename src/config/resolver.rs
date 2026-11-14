//! Invocation-local configuration cache for filesystem inputs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{CONFIG_FILE_NAME, Config, ConfigSource, env_config_path};
use crate::file_discovery::ExcludeFilter;

/// One selected configuration, shared by all directories that discover it.
pub struct ResolvedConfig {
    pub config: Config,
    pub exclude: ExcludeFilter,
}

/// Resolve once per directory and load once per selected config. Unlike the
/// language server's cache this lives for one invocation, so it needs no watcher
/// invalidation. Fallback exclusions stay anchored to the invocation's cwd.
pub struct DirectoryConfigs {
    cwd: PathBuf,
    explicit: Option<PathBuf>,
    no_config: bool,
    extra: Vec<String>,
    force: bool,
    directories: HashMap<PathBuf, Arc<ResolvedConfig>>,
    sources: HashMap<Option<PathBuf>, Arc<ResolvedConfig>>,
}

impl DirectoryConfigs {
    pub fn new(
        cwd: PathBuf,
        explicit: Option<PathBuf>,
        no_config: bool,
        extra: Vec<String>,
        force: bool,
    ) -> Self {
        Self {
            cwd,
            explicit,
            no_config,
            extra,
            force,
            directories: HashMap::new(),
            sources: HashMap::new(),
        }
    }

    pub fn resolve(&mut self, directory: &Path) -> Result<Arc<ResolvedConfig>, String> {
        let directory = if directory.is_absolute() {
            directory.to_path_buf()
        } else {
            self.cwd.join(directory)
        };
        if let Some(resolved) = self.directories.get(&directory) {
            return Ok(Arc::clone(resolved));
        }
        let resolved = if self.no_config {
            self.load(ConfigSource::None)?
        } else if let Some(path) = self.explicit.clone() {
            self.load(ConfigSource::Explicit(path))?
        } else {
            // Same canonical ancestor walk and repository boundary as
            // Config::discover (also used by LSP settings).
            let canonical = super::deepest_existing_ancestor(&directory);
            match canonical {
                Some(dir) => self.discover(&dir)?,
                None => self.fallback()?,
            }
        };
        self.directories.insert(directory, Arc::clone(&resolved));
        Ok(resolved)
    }

    fn discover(&mut self, directory: &Path) -> Result<Arc<ResolvedConfig>, String> {
        if let Some(resolved) = self.directories.get(directory) {
            return Ok(Arc::clone(resolved));
        }
        let candidate = directory.join(CONFIG_FILE_NAME);
        let resolved = if candidate.is_file() {
            self.load(ConfigSource::Discovered(candidate))?
        } else if directory.join(".git").exists() {
            self.fallback()?
        } else if let Some(parent) = directory.parent() {
            self.discover(parent)?
        } else {
            self.fallback()?
        };
        self.directories
            .insert(directory.to_path_buf(), Arc::clone(&resolved));
        Ok(resolved)
    }

    fn fallback(&mut self) -> Result<Arc<ResolvedConfig>, String> {
        self.load(
            env_config_path()
                .map(ConfigSource::Env)
                .unwrap_or(ConfigSource::None),
        )
    }

    fn load(&mut self, source: ConfigSource) -> Result<Arc<ResolvedConfig>, String> {
        let key = source.path().map(Path::to_path_buf);
        if let Some(resolved) = self.sources.get(&key) {
            return Ok(Arc::clone(resolved));
        }
        let config = match source.path() {
            Some(path) => Config::load_from(path).map_err(|err| err.to_string())?,
            None => Config::default(),
        };
        let exclude = config
            .exclude_filter(&source, &self.cwd, &self.extra)
            .map_err(|err| err.to_string())?
            .with_force_exclude(self.force);
        let resolved = Arc::new(ResolvedConfig { config, exclude });
        self.sources.insert(key, Arc::clone(&resolved));
        Ok(resolved)
    }

    /// Directory entries are governed by the parent config. Once admitted, a
    /// directory's own config governs its children. A config inside a pruned
    /// directory is never loaded. Explicit files bypass exclusions by default.
    pub fn includes(&mut self, path: &Path, is_dir: bool, explicit: bool) -> Result<bool, String> {
        let directory = path.parent().unwrap_or(&self.cwd).to_path_buf();
        let resolved = self.resolve(&directory)?;
        if explicit && !self.force {
            return Ok(true);
        }
        Ok(!resolved.exclude.excludes_entry(path, is_dir, explicit))
    }
}
