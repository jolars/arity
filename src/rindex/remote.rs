//! The downloadable CRAN symbol sidecar: a names-only, all-CRAN resolution tier
//! fetched lazily over the network and cached on disk.
//!
//! [`RemoteExports`] is a dynamic, owned twin of
//! [`BundledPackages`](crate::semantic::symbols::BundledPackages): a
//! `package → exported names` map with no signatures or help. It sits between the
//! locally harvested index (version-exact, full metadata) and the baked-in
//! bundled lists in the masking stack
//! ([`resolve_origin`](crate::rindex::provider::resolve_origin)), so referencing
//! an uninstalled CRAN package outside the bundled top-N still resolves its bare
//! and `pkg::` names instead of forcing whole-file `undefined-symbol`
//! suppression.
//!
//! Unlike the static layers it is populated at runtime — loaded from a disk
//! cache and topped up by per-package network fetches in the lint thread's
//! write-phase — and is held behind the salsa [`LibraryIndex`]'s `remote` field
//! at `Durability::HIGH`, so a keystroke never revalidates resolution merely
//! because the sidecar is present.
//!
//! [`LibraryIndex`]: crate::incremental::LibraryIndex

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::semantic::symbols::unbacktick;

/// Names-only export lists for packages fetched from the remote sidecar, keyed
/// by package name. Owned (not `&'static`) because it is built dynamically from
/// the disk cache and network fetches, unlike the compile-time bundled lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteExports {
    /// package → set of exported names.
    exports: HashMap<SmolStr, HashSet<SmolStr>>,
}

impl RemoteExports {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a package's export names, replacing any previous entry. An entry
    /// is created even when `names` is empty, so [`has_package`](Self::has_package)
    /// reports the package as known (resolution is then complete for it).
    pub fn insert_package<I>(&mut self, package: impl Into<SmolStr>, names: I)
    where
        I: IntoIterator<Item = SmolStr>,
    {
        self.exports
            .insert(package.into(), names.into_iter().collect());
    }

    /// Fold every entry of `other` into this set, replacing on collision. Used by
    /// the lint thread to merge a freshly-fetched batch into the live sidecar.
    pub fn merge_from(&mut self, other: RemoteExports) {
        self.exports.extend(other.exports);
    }

    /// True if the sidecar has an export list for `package`.
    pub fn has_package(&self, package: &str) -> bool {
        self.exports.contains_key(package)
    }

    /// True if the sidecar's list for `package` includes `name`.
    pub fn exports(&self, package: &str, name: &str) -> bool {
        self.exports
            .get(package)
            .is_some_and(|set| set.contains(unbacktick(name)))
    }

    /// Iterate a package's export names, if it is in the sidecar (for
    /// completion's member fallback when the package isn't locally harvested).
    pub fn package_exports(&self, package: &str) -> Option<impl Iterator<Item = &SmolStr>> {
        self.exports.get(package).map(|set| set.iter())
    }

    /// Number of packages currently cached, for diagnostics/tests.
    pub fn len(&self) -> usize {
        self.exports.len()
    }

    pub fn is_empty(&self) -> bool {
        self.exports.is_empty()
    }
}

/// Cap on a single fetched body, so a misconfigured URL can't exhaust memory.
/// The all-CRAN manifest is the largest payload and sits comfortably under this.
const MAX_BODY_BYTES: u64 = 64 * 1024 * 1024;

/// The sidecar layout version, embedded in URLs and the on-disk cache dir so a
/// future format change can coexist with old caches. Bump when the wire format
/// or disk schema changes incompatibly.
const SIDECAR_VERSION: &str = "v1";

/// How long a cached manifest is trusted before a conditional re-fetch. The
/// per-package files are immutable (keyed by version), so only the manifest —
/// the `pkg → current version` map — can go stale; this bounds that staleness.
const MANIFEST_TTL_SECS: u64 = 24 * 60 * 60;

/// HTTP 304: the conditionally-requested resource is unchanged.
const NOT_MODIFIED: u16 = 304;

#[derive(Debug)]
pub enum SidecarError {
    Http(String),
    Io(String),
}

impl std::fmt::Display for SidecarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SidecarError::Http(s) => write!(f, "sidecar request failed: {s}"),
            SidecarError::Io(s) => write!(f, "sidecar I/O error: {s}"),
        }
    }
}

impl std::error::Error for SidecarError {}

/// A response from the sidecar transport: status, body, and the `ETag` (if any),
/// so the caller can do conditional re-fetches and store the validator.
pub struct SidecarResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub etag: Option<String>,
}

/// How the sidecar reaches the network. Abstracted behind a trait so the fetch
/// logic is unit-testable offline with a stub.
pub trait SidecarTransport: Send + Sync {
    /// GET `url`, optionally conditional on `if_none_match` (the stored `ETag`).
    /// A `304` is returned as [`SidecarResponse`] with that status and an empty
    /// body; transport-level failures (including 404) are `Err`.
    fn get(&self, url: &str, if_none_match: Option<&str>) -> Result<SidecarResponse, SidecarError>;
}

/// The production transport: a `ureq` agent with a short timeout. Static files
/// over HTTPS from the sidecar CDN; no auth, no cookies.
pub struct HttpTransport {
    agent: ureq::Agent,
}

impl HttpTransport {
    pub fn new() -> Self {
        // `http_status_as_error(false)` keeps non-2xx responses on the `Ok` path
        // so a conditional request's `304` still carries its headers; the status
        // is classified below instead.
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl SidecarTransport for HttpTransport {
    fn get(&self, url: &str, if_none_match: Option<&str>) -> Result<SidecarResponse, SidecarError> {
        let mut req = self.agent.get(url);
        if let Some(tag) = if_none_match {
            req = req.header("If-None-Match", tag);
        }
        let resp = req.call().map_err(|e| SidecarError::Http(e.to_string()))?;
        let (parts, body) = resp.into_parts();
        let status = parts.status.as_u16();
        let etag = parts
            .headers
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        // 304 is the expected "unchanged" answer to a conditional request, not a
        // failure; any other non-2xx is.
        if status == NOT_MODIFIED {
            return Ok(SidecarResponse {
                status: NOT_MODIFIED,
                body: Vec::new(),
                etag,
            });
        }
        if !parts.status.is_success() {
            return Err(SidecarError::Http(format!("HTTP {status}")));
        }

        let body = body
            .into_with_config()
            .limit(MAX_BODY_BYTES)
            .read_to_vec()
            .map_err(|e| SidecarError::Io(e.to_string()))?;
        Ok(SidecarResponse {
            status: 200,
            body,
            etag,
        })
    }
}

/// One package's entry in the manifest: its current version and an optional
/// content hash. The version keys every per-package fetch and disk-cache entry
/// (leaving room for pin-aware version selection later); `sha256`, when present,
/// is the SHA-256 of the package's export file — a reserved slot for future
/// integrity verification (the manifest is fetched over TLS and is the trust
/// anchor; per-file verification is **not** performed yet).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackageEntry {
    pub version: SmolStr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// The package → entry map published by the sidecar. Fetched with a TTL +
/// conditional (`ETag`) re-fetch and cached on disk.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub packages: BTreeMap<SmolStr, PackageEntry>,
}

impl Manifest {
    /// The current version the sidecar publishes for `package`, if any.
    pub fn version_of(&self, package: &str) -> Option<&SmolStr> {
        self.packages.get(package).map(|e| &e.version)
    }
}

/// On-disk validators for the cached manifest: the `ETag` to send as
/// `If-None-Match`, and when it was last confirmed fresh (unix seconds), for the
/// TTL. Stored beside the manifest as `manifest.meta.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ManifestMeta {
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    fetched_at: u64,
}

/// The downloadable sidecar: a base URL plus an on-disk cache, fetching
/// names-only export lists per package on demand. Stateful only in that it
/// memoizes the manifest in memory after first load.
///
/// Disk layout under the cache root:
///
/// ```text
/// {cache_root}/sidecar/{SIDECAR_VERSION}/
///     manifest.json           # cached pkg -> version map
///     {pkg}@{ver}.txt          # one export name per line
/// ```
pub struct Sidecar {
    base_url: String,
    dir: PathBuf,
    transport: Box<dyn SidecarTransport>,
    manifest: Option<Manifest>,
    /// Wall-clock (unix seconds) used for the manifest TTL. Injected so the
    /// freshness decision is deterministic in tests.
    now: u64,
}

impl Sidecar {
    /// Build a sidecar for `base_url`, caching under `{cache_root}/sidecar/...`.
    /// `now` is unix seconds, used for the manifest TTL.
    pub fn new(
        base_url: impl Into<String>,
        cache_root: &Path,
        transport: Box<dyn SidecarTransport>,
        now: u64,
    ) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        Self {
            base_url,
            dir: cache_root.join("sidecar").join(SIDECAR_VERSION),
            transport,
            manifest: None,
            now,
        }
    }

    /// Convenience constructor using the live [`HttpTransport`] and the system
    /// clock.
    pub fn http(base_url: impl Into<String>, cache_root: &Path) -> Self {
        Self::new(
            base_url,
            cache_root,
            Box::new(HttpTransport::new()),
            now_unix(),
        )
    }

    fn manifest_url(&self) -> String {
        format!("{}/{SIDECAR_VERSION}/manifest.json", self.base_url)
    }

    fn package_url(&self, package: &str, version: &str) -> String {
        format!(
            "{}/{SIDECAR_VERSION}/{package}/{version}.txt",
            self.base_url
        )
    }

    fn manifest_path(&self) -> PathBuf {
        self.dir.join("manifest.json")
    }

    fn meta_path(&self) -> PathBuf {
        self.dir.join("manifest.meta.json")
    }

    fn package_path(&self, package: &str, version: &str) -> PathBuf {
        self.dir.join(format!("{package}@{version}.txt"))
    }

    /// Load every export list already present on disk into a [`RemoteExports`],
    /// without any network access. Used to warm the resolver from the disk cache
    /// at startup. Reads the on-disk manifest to learn the cached version per
    /// package; a `{pkg}@{ver}.txt` whose version isn't in the manifest is
    /// ignored (treated as stale).
    pub fn load_cached(&self) -> RemoteExports {
        let mut out = RemoteExports::new();
        let Some(manifest) = read_disk_manifest(&self.manifest_path()) else {
            return out;
        };
        for (pkg, entry) in &manifest.packages {
            if let Some(names) = read_exports_file(&self.package_path(pkg, &entry.version)) {
                out.insert_package(pkg.clone(), names);
            }
        }
        out
    }

    /// The manifest, loaded once and memoized. Resolution order: in-memory → a
    /// disk copy still within [`MANIFEST_TTL_SECS`] → a conditional (`ETag`)
    /// network re-fetch. A `304` keeps the disk copy and just refreshes the TTL;
    /// a network failure falls back to the (stale) disk copy rather than losing
    /// resolution. Returns `None` only when there is neither a cached nor a
    /// fetchable manifest.
    pub fn manifest(&mut self) -> Option<&Manifest> {
        if self.manifest.is_some() {
            return self.manifest.as_ref();
        }
        let now = self.now;
        let disk = read_disk_manifest(&self.manifest_path());
        let meta = read_manifest_meta(&self.meta_path());
        let fresh = disk.is_some()
            && meta
                .as_ref()
                .is_some_and(|m| now.saturating_sub(m.fetched_at) < MANIFEST_TTL_SECS);
        if fresh {
            self.manifest = disk;
            return self.manifest.as_ref();
        }

        // Stale or absent: (conditionally) fetch. Only send `If-None-Match` when
        // we still hold the disk copy that a `304` would validate.
        let inm = disk
            .as_ref()
            .and_then(|_| meta.as_ref().and_then(|m| m.etag.clone()));
        match self.transport.get(&self.manifest_url(), inm.as_deref()) {
            Ok(resp) if resp.status == NOT_MODIFIED => {
                let etag = resp.etag.or(inm);
                let _ = write_manifest_meta(
                    &self.meta_path(),
                    &ManifestMeta {
                        etag,
                        fetched_at: now,
                    },
                );
                self.manifest = disk;
            }
            Ok(resp) => {
                let decoded = maybe_gunzip(resp.body);
                match serde_json::from_slice::<Manifest>(&decoded) {
                    Ok(m) => {
                        let _ = write_atomic(&self.manifest_path(), &decoded);
                        let _ = write_manifest_meta(
                            &self.meta_path(),
                            &ManifestMeta {
                                etag: resp.etag,
                                fetched_at: now,
                            },
                        );
                        self.manifest = Some(m);
                    }
                    // Unparseable body: keep the stale disk copy if we have one.
                    Err(_) => self.manifest = disk,
                }
            }
            // Network down: degrade to the stale disk copy rather than nothing.
            Err(_) => self.manifest = disk,
        }
        self.manifest.as_ref()
    }

    /// Resolve `package`'s export names: disk cache first, then a network fetch
    /// (which is written to the disk cache). Returns `None` when the package is
    /// absent from the manifest or unreachable. The version comes from the
    /// manifest (the current-CRAN snapshot).
    pub fn package_names(&mut self, package: &str) -> Option<Vec<SmolStr>> {
        let version = self.manifest()?.version_of(package)?.clone();
        if let Some(names) = read_exports_file(&self.package_path(package, &version)) {
            return Some(names);
        }
        // Per-package files are immutable (version-keyed), so the fetch is plain,
        // not conditional.
        let resp = self
            .transport
            .get(&self.package_url(package, &version), None)
            .ok()?;
        let text = String::from_utf8(maybe_gunzip(resp.body)).ok()?;
        let names = parse_exports(&text);
        let _ = write_atomic(&self.package_path(package, &version), text.as_bytes());
        Some(names)
    }
}

/// Unix seconds now, for the manifest TTL. Saturates to 0 before the epoch.
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Read the manifest validators (`ETag` + last-fresh time), if present.
fn read_manifest_meta(path: &Path) -> Option<ManifestMeta> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Persist the manifest validators beside the manifest.
fn write_manifest_meta(path: &Path, meta: &ManifestMeta) -> Result<(), SidecarError> {
    let json = serde_json::to_vec(meta).map_err(|e| SidecarError::Io(e.to_string()))?;
    write_atomic(path, &json)
}

/// Read and parse the on-disk manifest, if present and valid.
fn read_disk_manifest(path: &Path) -> Option<Manifest> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Read a `{pkg}@{ver}.txt` export file into a name list, if present.
fn read_exports_file(path: &Path) -> Option<Vec<SmolStr>> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(parse_exports(&text))
}

/// Parse an export-names file: one name per line, `#` comments and blanks
/// ignored. Mirrors the `cran/exports.txt` per-section body format.
fn parse_exports(text: &str) -> Vec<SmolStr> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(SmolStr::new)
        .collect()
}

/// Gunzip `bytes` when they carry the gzip magic, else return them unchanged.
/// Lets the sidecar serve either gzipped or plain static files transparently.
fn maybe_gunzip(bytes: Vec<u8>) -> Vec<u8> {
    if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        let mut out = Vec::new();
        if GzDecoder::new(&bytes[..]).read_to_end(&mut out).is_ok() {
            return out;
        }
    }
    bytes
}

/// Write `bytes` to `path` via a temp-file-then-rename, creating parents. Best
/// effort: a cache write failure is non-fatal (the value is still returned).
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), SidecarError> {
    let dir = path
        .parent()
        .ok_or_else(|| SidecarError::Io("no parent directory".into()))?;
    std::fs::create_dir_all(dir).map_err(|e| SidecarError::Io(e.to_string()))?;
    let mut tmp =
        tempfile::NamedTempFile::new_in(dir).map_err(|e| SidecarError::Io(e.to_string()))?;
    use std::io::Write as _;
    tmp.write_all(bytes)
        .map_err(|e| SidecarError::Io(e.to_string()))?;
    tmp.persist(path)
        .map_err(|e| SidecarError::Io(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[&str]) -> Vec<SmolStr> {
        items.iter().copied().map(SmolStr::new).collect()
    }

    #[test]
    fn insert_then_query() {
        let mut remote = RemoteExports::new();
        remote.insert_package("cli", names(&["cli_alert", "cli_abort"]));
        assert!(remote.has_package("cli"));
        assert!(remote.exports("cli", "cli_alert"));
        assert!(!remote.exports("cli", "not_an_export"));
        assert!(!remote.has_package("absent"));
        assert!(!remote.exports("absent", "anything"));
    }

    #[test]
    fn empty_package_is_known_but_exports_nothing() {
        // A package with no exports is still "known" — resolution is complete for
        // it, so a name attributed to it is genuinely undefined, not un-indexed.
        let mut remote = RemoteExports::new();
        remote.insert_package("empty", names(&[]));
        assert!(remote.has_package("empty"));
        assert!(!remote.exports("empty", "anything"));
    }

    #[test]
    fn insert_replaces_previous_entry() {
        let mut remote = RemoteExports::new();
        remote.insert_package("pkg", names(&["old"]));
        remote.insert_package("pkg", names(&["new"]));
        assert!(!remote.exports("pkg", "old"));
        assert!(remote.exports("pkg", "new"));
    }

    #[test]
    fn package_exports_iterates_names() {
        let mut remote = RemoteExports::new();
        remote.insert_package("pkg", names(&["a", "b"]));
        let mut got: Vec<String> = remote
            .package_exports("pkg")
            .expect("known")
            .map(|s| s.to_string())
            .collect();
        got.sort();
        assert_eq!(got, vec!["a".to_string(), "b".to_string()]);
        assert!(remote.package_exports("absent").is_none());
    }

    /// A transport backed by an in-memory URL → bytes map; every response is a
    /// plain `200` with no `ETag`.
    struct StubTransport {
        responses: HashMap<String, Vec<u8>>,
    }

    impl StubTransport {
        fn new(responses: &[(&str, &str)]) -> Self {
            Self {
                responses: responses
                    .iter()
                    .map(|(u, b)| (u.to_string(), b.as_bytes().to_vec()))
                    .collect(),
            }
        }
    }

    impl SidecarTransport for StubTransport {
        fn get(&self, url: &str, _inm: Option<&str>) -> Result<SidecarResponse, SidecarError> {
            self.responses
                .get(url)
                .cloned()
                .map(|body| SidecarResponse {
                    status: 200,
                    body,
                    etag: None,
                })
                .ok_or_else(|| SidecarError::Http(format!("404 {url}")))
        }
    }

    const BASE: &str = "https://sidecar.example/cran";
    const NOW: u64 = 1_000_000;

    fn manifest_json() -> &'static str {
        r#"{"packages":{"tinytable":{"version":"0.6.1"},"cli":{"version":"3.6.1"}}}"#
    }

    #[test]
    fn fetches_and_caches_package_names() {
        let tmp = tempfile::tempdir().unwrap();
        let stub = StubTransport::new(&[
            (&format!("{BASE}/v1/manifest.json"), manifest_json()),
            (
                &format!("{BASE}/v1/tinytable/0.6.1.txt"),
                "tt\ntheme_tt\n# a comment\n\nformat_tt\n",
            ),
        ]);
        let mut sidecar = Sidecar::new(BASE, tmp.path(), Box::new(stub), NOW);

        let got = sidecar.package_names("tinytable").expect("fetched");
        let got: Vec<String> = got.iter().map(|s| s.to_string()).collect();
        assert!(got.contains(&"tt".to_string()), "{got:?}");
        assert!(got.contains(&"format_tt".to_string()), "{got:?}");
        assert!(!got.iter().any(|n| n.starts_with('#')), "comments stripped");

        // The export file landed in the disk cache, keyed by version.
        assert!(tmp.path().join("sidecar/v1/tinytable@0.6.1.txt").exists());
    }

    #[test]
    fn second_lookup_is_a_disk_cache_hit() {
        let tmp = tempfile::tempdir().unwrap();
        let stub = StubTransport::new(&[
            (&format!("{BASE}/v1/manifest.json"), manifest_json()),
            (
                &format!("{BASE}/v1/cli/3.6.1.txt"),
                "cli_alert\ncli_abort\n",
            ),
        ]);
        let mut sidecar = Sidecar::new(BASE, tmp.path(), Box::new(stub), NOW);
        assert!(sidecar.package_names("cli").is_some());

        // A fresh sidecar over the same cache dir, within the manifest TTL and
        // with a transport that panics if hit, still resolves entirely from disk.
        struct NoNet;
        impl SidecarTransport for NoNet {
            fn get(&self, url: &str, _inm: Option<&str>) -> Result<SidecarResponse, SidecarError> {
                panic!("unexpected network access: {url}");
            }
        }
        let mut offline = Sidecar::new(BASE, tmp.path(), Box::new(NoNet), NOW + 10);
        let got = offline.package_names("cli").expect("from disk");
        assert!(got.iter().any(|n| n == "cli_alert"), "{got:?}");
        // And `load_cached` warms a RemoteExports straight off disk.
        let warmed = offline.load_cached();
        assert!(warmed.exports("cli", "cli_abort"));
    }

    #[test]
    fn unknown_package_yields_none_without_package_fetch() {
        let tmp = tempfile::tempdir().unwrap();
        let stub = StubTransport::new(&[(&format!("{BASE}/v1/manifest.json"), manifest_json())]);
        let mut sidecar = Sidecar::new(BASE, tmp.path(), Box::new(stub), NOW);
        assert!(sidecar.package_names("not_on_cran_xyz").is_none());
    }

    #[test]
    fn gunzips_gzipped_bodies() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Write as _;

        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(b"gt\nplotgg\n").unwrap();
        let gz = enc.finish().unwrap();

        // A transport that returns gzip-compressed bytes for the package file.
        struct GzStub {
            manifest: Vec<u8>,
            pkg: Vec<u8>,
        }
        impl SidecarTransport for GzStub {
            fn get(&self, url: &str, _inm: Option<&str>) -> Result<SidecarResponse, SidecarError> {
                let body = if url.ends_with("manifest.json") {
                    self.manifest.clone()
                } else if url.ends_with("ggplot2/3.5.1.txt") {
                    self.pkg.clone()
                } else {
                    return Err(SidecarError::Http("404".into()));
                };
                Ok(SidecarResponse {
                    status: 200,
                    body,
                    etag: None,
                })
            }
        }
        let tmp = tempfile::tempdir().unwrap();
        let transport = GzStub {
            manifest: br#"{"packages":{"ggplot2":{"version":"3.5.1"}}}"#.to_vec(),
            pkg: gz,
        };
        let mut sidecar = Sidecar::new(BASE, tmp.path(), Box::new(transport), NOW);
        let got = sidecar.package_names("ggplot2").expect("decoded");
        assert!(got.iter().any(|n| n == "plotgg"), "{got:?}");
    }

    /// A manifest transport that records the `If-None-Match` it sees and answers
    /// `304` when the client's validator matches `etag`, else `200` with `body`.
    struct EtagStub {
        etag: String,
        body: Vec<u8>,
        seen_inm: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
    }

    impl SidecarTransport for EtagStub {
        fn get(&self, url: &str, inm: Option<&str>) -> Result<SidecarResponse, SidecarError> {
            assert!(
                url.ends_with("manifest.json"),
                "only the manifest is fetched"
            );
            self.seen_inm.lock().unwrap().push(inm.map(str::to_string));
            if inm == Some(self.etag.as_str()) {
                return Ok(SidecarResponse {
                    status: NOT_MODIFIED,
                    body: Vec::new(),
                    etag: Some(self.etag.clone()),
                });
            }
            Ok(SidecarResponse {
                status: 200,
                body: self.body.clone(),
                etag: Some(self.etag.clone()),
            })
        }
    }

    #[test]
    fn stale_manifest_revalidates_with_etag_and_304() {
        let tmp = tempfile::tempdir().unwrap();
        let body = br#"{"packages":{"tinytable":{"version":"0.6.1"}}}"#.to_vec();

        // First load (cold): a plain 200 stores the manifest + meta (etag, time).
        let seen0 = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut s1 = Sidecar::new(
            BASE,
            tmp.path(),
            Box::new(EtagStub {
                etag: "etag-v1".into(),
                body: body.clone(),
                seen_inm: seen0.clone(),
            }),
            NOW,
        );
        assert_eq!(
            s1.manifest()
                .and_then(|m| m.version_of("tinytable"))
                .map(|v| v.as_str()),
            Some("0.6.1")
        );
        assert_eq!(
            seen0.lock().unwrap().as_slice(),
            &[None],
            "first load is unconditional"
        );

        // Past the TTL: a new sidecar must revalidate conditionally and accept 304.
        let seen1 = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut s2 = Sidecar::new(
            BASE,
            tmp.path(),
            Box::new(EtagStub {
                etag: "etag-v1".into(),
                body,
                seen_inm: seen1.clone(),
            }),
            NOW + MANIFEST_TTL_SECS + 1,
        );
        // Still resolves — kept the disk copy the 304 validated.
        assert_eq!(
            s2.manifest()
                .and_then(|m| m.version_of("tinytable"))
                .map(|v| v.as_str()),
            Some("0.6.1")
        );
        assert_eq!(
            seen1.lock().unwrap().as_slice(),
            &[Some("etag-v1".to_string())],
            "revalidation sent the stored ETag as If-None-Match"
        );
    }

    #[test]
    fn stale_manifest_with_changed_etag_picks_up_new_version() {
        let tmp = tempfile::tempdir().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        // Cold load: tinytable 0.6.1 under etag-v1.
        let mut s1 = Sidecar::new(
            BASE,
            tmp.path(),
            Box::new(EtagStub {
                etag: "etag-v1".into(),
                body: br#"{"packages":{"tinytable":{"version":"0.6.1"}}}"#.to_vec(),
                seen_inm: seen.clone(),
            }),
            NOW,
        );
        assert!(s1.manifest().is_some());

        // Past the TTL the server has a new etag + a bumped version; the
        // conditional GET misses (etags differ) and the new manifest is adopted.
        let mut s2 = Sidecar::new(
            BASE,
            tmp.path(),
            Box::new(EtagStub {
                etag: "etag-v2".into(),
                body: br#"{"packages":{"tinytable":{"version":"0.7.0"}}}"#.to_vec(),
                seen_inm: seen.clone(),
            }),
            NOW + MANIFEST_TTL_SECS + 1,
        );
        assert_eq!(
            s2.manifest()
                .and_then(|m| m.version_of("tinytable"))
                .map(|v| v.as_str()),
            Some("0.7.0"),
            "a changed manifest is adopted on revalidation"
        );
    }

    #[test]
    fn manifest_accepts_optional_sha256_slot() {
        // The integrity slot parses (verification is not wired yet).
        let json = r#"{"packages":{"cli":{"version":"3.6.1","sha256":"deadbeef"}}}"#;
        let manifest: Manifest = serde_json::from_slice(json.as_bytes()).expect("parse");
        let entry = manifest.packages.get("cli").expect("cli present");
        assert_eq!(entry.version.as_str(), "3.6.1");
        assert_eq!(entry.sha256.as_deref(), Some("deadbeef"));
    }
}
