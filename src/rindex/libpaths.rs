//! Discover R library locations from the filesystem, with one cheap R probe.
//!
//! `.libPaths()` is computed by R from environment variables, platform
//! defaults, and project overlays (`renv`/`packrat`). We replicate the parts we
//! can read off disk, in priority order, and probe each candidate for the
//! requested package. The one place we ask R is for `R_HOME` when it is absent
//! from the environment: `R RHOME` prints the home directory and evaluates no
//! user code, so an editor-launched server (which often doesn't inherit
//! `R_HOME`) can still find the *system* library where the default packages
//! (`base`, `stats`, …) live. When discovery still misses (exotic layouts,
//! custom `.Renviron`, R not on `PATH`), the user points us at the right
//! directory via the `[index].library-paths` config (the highest-priority
//! source).

use std::path::{Path, PathBuf};

/// An ordered set of candidate library directories.
#[derive(Debug, Clone, Default)]
pub struct LibrarySearch {
    dirs: Vec<PathBuf>,
}

impl LibrarySearch {
    /// Assemble the search path from the real environment + filesystem. When
    /// `R_HOME` is absent, fall back to probing `R RHOME` so the system library
    /// is still found (see the module doc).
    pub fn discover(project_root: Option<&Path>, configured: &[PathBuf]) -> Self {
        // Resolve `R_HOME` once: the environment wins, then a `R RHOME` probe.
        let r_home = std::env::var("R_HOME")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(probe_r_home);
        let env = move |k: &str| match k {
            "R_HOME" => r_home.clone(),
            other => std::env::var(other).ok(),
        };
        Self::assemble(project_root, configured, &env, home_dir())
    }

    /// All candidate directories, in priority order, deduplicated.
    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    /// The directory of an installed `package`, i.e. the first candidate that
    /// contains `package/DESCRIPTION`.
    pub fn find_package(&self, package: &str) -> Option<PathBuf> {
        self.dirs.iter().find_map(|dir| {
            let candidate = dir.join(package);
            candidate.join("DESCRIPTION").is_file().then_some(candidate)
        })
    }

    /// Core assembly, parameterized over env lookup + home dir for testing.
    fn assemble(
        project_root: Option<&Path>,
        configured: &[PathBuf],
        env: &dyn Fn(&str) -> Option<String>,
        home: Option<PathBuf>,
    ) -> Self {
        let mut dirs: Vec<PathBuf> = Vec::new();
        let push = |dirs: &mut Vec<PathBuf>, p: PathBuf| {
            if !dirs.contains(&p) {
                dirs.push(p);
            }
        };

        // 1. Explicit configuration wins.
        for p in configured {
            push(&mut dirs, p.clone());
        }

        // 2. Project overlays: renv / packrat.
        if let Some(root) = project_root {
            for p in project_overlay_dirs(root) {
                push(&mut dirs, p);
            }
        }

        // 3. Environment variables (highest R priority among these is
        //    R_LIBS_USER, then R_LIBS_SITE, then R_LIBS).
        for key in ["R_LIBS_USER", "R_LIBS_SITE", "R_LIBS"] {
            if let Some(val) = env(key) {
                for p in split_path_list(&val) {
                    push(&mut dirs, p);
                }
            }
        }

        // 4. Platform user-library defaults.
        if let Some(home) = home.as_ref() {
            for p in default_user_libs(home) {
                push(&mut dirs, p);
            }
        }

        // 5. System library under R_HOME.
        if let Some(r_home) = env("R_HOME")
            && !r_home.is_empty()
        {
            push(&mut dirs, PathBuf::from(r_home).join("library"));
        }

        LibrarySearch { dirs }
    }
}

/// Ask R for its home directory. `R RHOME` only prints `R.home()` — it does not
/// start an R session or evaluate user code — so it stays within the
/// "no R evaluation" tenet while letting us locate the *system* library (where
/// the default packages live) when `R_HOME` is missing from the environment.
/// Returns `None` if R isn't on `PATH` or the probe fails.
fn probe_r_home() -> Option<String> {
    let output = std::process::Command::new("R").arg("RHOME").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let home = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!home.is_empty()).then_some(home)
}

/// Split an `R_LIBS*`-style path list on the platform separator.
fn split_path_list(value: &str) -> Vec<PathBuf> {
    let sep = if cfg!(windows) { ';' } else { ':' };
    value
        .split(sep)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// `renv` and `packrat` library directories under a project root. Both nest the
/// real package directories a couple of levels deep and the exact layout has
/// shifted across versions, so we add the container plus shallow descendants
/// and let `find_package` probe.
fn project_overlay_dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for container in [root.join("renv/library"), root.join("packrat/lib")] {
        if container.is_dir() {
            out.push(container.clone());
            collect_shallow(&container, 2, &mut out);
        }
    }
    out
}

/// Add directories up to `depth` levels below `base` (not `base` itself).
fn collect_shallow(base: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.push(path.clone());
            collect_shallow(&path, depth - 1, out);
        }
    }
}

/// Platform default user-library locations. We can't know the exact
/// platform/R-version subdirectory without R, so we probe the common roots
/// (`~/.R/library`, and any `~/R/*-library/*`).
fn default_user_libs(home: &Path) -> Vec<PathBuf> {
    let mut out = vec![home.join(".R/library")];
    // ~/R/<platform>-library/<x.y>
    let r_root = home.join("R");
    if r_root.is_dir() {
        collect_shallow(&r_root, 2, &mut out);
    }
    out
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_from<'a>(map: &'a HashMap<&str, &str>) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| map.get(k).map(|s| s.to_string())
    }

    #[test]
    fn splits_env_path_list_unix() {
        // This test asserts the unix separator; gate to non-windows.
        if cfg!(windows) {
            return;
        }
        let dirs = split_path_list("/a/lib:/b/lib::/c/lib");
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/a/lib"),
                PathBuf::from("/b/lib"),
                PathBuf::from("/c/lib"),
            ]
        );
    }

    #[test]
    fn configured_paths_come_first() {
        let env = HashMap::new();
        let search =
            LibrarySearch::assemble(None, &[PathBuf::from("/custom/lib")], &env_from(&env), None);
        assert_eq!(search.dirs().first(), Some(&PathBuf::from("/custom/lib")));
    }

    #[test]
    fn reads_r_libs_env_in_priority_order() {
        // Join with the platform separator so `split_path_list` round-trips
        // (`;` on Windows, `:` elsewhere).
        let site = if cfg!(windows) {
            "/site/a;/site/b"
        } else {
            "/site/a:/site/b"
        };
        let mut env = HashMap::new();
        env.insert("R_LIBS_USER", "/user/lib");
        env.insert("R_LIBS_SITE", site);
        let search = LibrarySearch::assemble(None, &[], &env_from(&env), None);
        let dirs = search.dirs();
        assert!(dirs.contains(&PathBuf::from("/user/lib")));
        assert!(dirs.contains(&PathBuf::from("/site/a")));
        assert!(dirs.contains(&PathBuf::from("/site/b")));
        // User lib precedes site lib.
        let ui = dirs
            .iter()
            .position(|d| d == Path::new("/user/lib"))
            .unwrap();
        let si = dirs.iter().position(|d| d == Path::new("/site/a")).unwrap();
        assert!(ui < si);
    }

    #[test]
    fn appends_r_home_system_library() {
        // `R_HOME` (env or `R RHOME` probe) contributes `$R_HOME/library`, the
        // system library where the default packages live.
        let mut env = HashMap::new();
        env.insert("R_HOME", "/opt/R");
        let search = LibrarySearch::assemble(None, &[], &env_from(&env), None);
        assert!(
            search.dirs().contains(&PathBuf::from("/opt/R/library")),
            "system library missing from {:?}",
            search.dirs()
        );
    }

    #[test]
    fn finds_package_with_description() {
        let tmp = tempfile::tempdir().unwrap();
        let libdir = tmp.path().join("lib");
        let pkgdir = libdir.join("magrittr");
        std::fs::create_dir_all(&pkgdir).unwrap();
        std::fs::write(pkgdir.join("DESCRIPTION"), "Package: magrittr\n").unwrap();

        let mut env = HashMap::new();
        let libdir_str = libdir.to_string_lossy().into_owned();
        env.insert("R_LIBS", libdir_str.as_str());
        let search = LibrarySearch::assemble(None, &[], &env_from(&env), None);
        assert_eq!(search.find_package("magrittr"), Some(pkgdir));
        assert_eq!(search.find_package("nonexistent"), None);
    }
}
