//! Pluggable asset fetching for media references.
//!
//! `markdoc-pdf` doesn't speak Arca directly; it goes through this trait
//! so that any fetch backend (filesystem, HTTP, in-process Arca client,
//! pre-bundled in-memory cache) can be plugged in. Default deployments
//! use [`FsAssetResolver`] for local-path references; an Arca-backed
//! resolver can ship later as a separate impl when the contract lands.
//!
//! The renderer treats fetch failures as soft errors — the image is
//! replaced by a placeholder rather than aborting the PDF generation.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("asset not found: {0}")]
    NotFound(String),
    #[error("asset fetch failed for {uri}: {source}")]
    Io {
        uri: String,
        #[source]
        source: std::io::Error,
    },
    #[error("scheme {scheme} not supported by this resolver")]
    UnsupportedScheme { scheme: String },
    #[error("{0}")]
    Other(String),
}

pub trait AssetResolver: Send + Sync {
    /// Fetch the bytes referenced by `uri`. The URI can be a relative or
    /// absolute path, an `http://` / `https://` URL, an `arca://` reference,
    /// or anything else a caller-supplied resolver knows how to handle.
    fn fetch(&self, uri: &str) -> Result<Vec<u8>, AssetError>;

    /// Resolve a bare asset `id` (no path, no extension — e.g. an Arca
    /// UUID) to a URI that [`fetch`](Self::fetch) can load. A filesystem
    /// resolver searches its root recursively for a file whose stem matches;
    /// resolvers that can't map an id return `None` (the default). Used as a
    /// fallback for `{% media id="…" /%}` when the id isn't a file sitting
    /// directly in the asset root.
    fn resolve_id(&self, _id: &str) -> Option<String> {
        None
    }
}

/// Resolver that returns `NotFound` for every URI. The default when the
/// caller hasn't wired anything else; lets media-bearing documents render
/// as placeholders without configuration.
#[derive(Debug, Default)]
pub struct NullAssetResolver;

impl AssetResolver for NullAssetResolver {
    fn fetch(&self, uri: &str) -> Result<Vec<u8>, AssetError> {
        Err(AssetError::NotFound(uri.to_string()))
    }
}

/// Resolver that loads files from a filesystem root. URIs are joined to
/// `root` as relative paths; absolute paths and `file://` URIs are also
/// honored. Other schemes (`http`, `https`, `arca`, …) error.
pub struct FsAssetResolver {
    pub root: PathBuf,
    /// Lazily-built index of `<stem>` → path (relative to `root`) for every
    /// image file anywhere under `root`, so `{% media id="…" /%}` resolves
    /// even when the asset sits in a sub-folder. Built on first `resolve_id`.
    id_index: OnceLock<HashMap<String, PathBuf>>,
}

impl FsAssetResolver {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            id_index: OnceLock::new(),
        }
    }
}

impl AssetResolver for FsAssetResolver {
    fn fetch(&self, uri: &str) -> Result<Vec<u8>, AssetError> {
        let path = if let Some(rest) = uri.strip_prefix("file://") {
            PathBuf::from(rest)
        } else if uri.contains("://") {
            // Reject other schemes explicitly so the caller knows to
            // wrap us in a composite resolver.
            let scheme = uri.split("://").next().unwrap_or("?").to_string();
            return Err(AssetError::UnsupportedScheme { scheme });
        } else {
            let p = PathBuf::from(uri);
            if p.is_absolute() {
                p
            } else {
                self.root.join(p)
            }
        };
        std::fs::read(&path).map_err(|e| AssetError::Io {
            uri: uri.to_string(),
            source: e,
        })
    }

    fn resolve_id(&self, id: &str) -> Option<String> {
        let index = self.id_index.get_or_init(|| build_id_index(&self.root));
        index.get(id).map(|rel| rel.to_string_lossy().into_owned())
    }
}

/// Walk `root` recursively and index image files by their stem (the file
/// name without extension), keyed to the path relative to `root` — so the
/// result feeds straight back into [`FsAssetResolver::fetch`]. First match
/// wins on a stem collision (rare for UUID-named assets). Symlinked
/// directories are not followed, so the walk cannot loop.
fn build_id_index(root: &Path) -> HashMap<String, PathBuf> {
    const IMAGE_EXTS: &[&str] = &["webp", "png", "jpg", "jpeg", "gif", "svg"];
    let mut map: HashMap<String, PathBuf> = HashMap::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                let is_image = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
                    .unwrap_or(false);
                if is_image
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                    && let Ok(rel) = path.strip_prefix(root)
                {
                    map.entry(stem.to_string())
                        .or_insert_with(|| rel.to_path_buf());
                }
            }
        }
    }
    map
}

/// HTTP/HTTPS asset resolver. Performs a synchronous GET and returns
/// the response body. Other schemes return `UnsupportedScheme` so this
/// can sit in a `CompositeAssetResolver` alongside FS/Arca.
///
/// Defaults: 10-second timeout, 64 MiB max body, no per-request retry.
/// Use [`HttpAssetResolver::with_agent`] to customise.
pub struct HttpAssetResolver {
    agent: ureq::Agent,
    /// Cap on response body size to avoid loading hostile servers'
    /// runaway responses.
    pub max_body_bytes: usize,
}

impl HttpAssetResolver {
    pub fn new() -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(10)))
            .build()
            .into();
        Self {
            agent,
            max_body_bytes: 64 * 1024 * 1024,
        }
    }

    pub fn with_agent(agent: ureq::Agent) -> Self {
        Self {
            agent,
            max_body_bytes: 64 * 1024 * 1024,
        }
    }

    pub fn with_max_body_bytes(mut self, n: usize) -> Self {
        self.max_body_bytes = n;
        self
    }
}

impl Default for HttpAssetResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetResolver for HttpAssetResolver {
    fn fetch(&self, uri: &str) -> Result<Vec<u8>, AssetError> {
        if !(uri.starts_with("http://") || uri.starts_with("https://")) {
            let scheme = uri.split("://").next().unwrap_or("?").to_string();
            return Err(AssetError::UnsupportedScheme { scheme });
        }
        let mut resp = self
            .agent
            .get(uri)
            .call()
            .map_err(|e| AssetError::Other(format!("HTTP fetch {uri}: {e}")))?;
        let mut buf: Vec<u8> = Vec::new();
        let mut reader = resp
            .body_mut()
            .as_reader()
            .take(self.max_body_bytes as u64 + 1);
        std::io::copy(&mut reader, &mut buf).map_err(|e| AssetError::Io {
            uri: uri.to_string(),
            source: e,
        })?;
        if buf.len() > self.max_body_bytes {
            return Err(AssetError::Other(format!(
                "Response body exceeds max_body_bytes ({})",
                self.max_body_bytes
            )));
        }
        Ok(buf)
    }
}

/// Resolver for the Adeptus / Arca asset URI scheme:
///   - `arca://{asset_id}`              → `GET {base}/download/{asset_id}`
///   - `arca://{asset_id}/{derivative}` → `GET {base}/download/{asset_id}/{derivative}`
///
/// Authenticates by sending `X-Subject-Id` (the Ory Oathkeeper-injected
/// subject identifier the platform uses everywhere). The id is the
/// caller's choice — typically the PDF generator's service account or
/// the requesting user.
pub struct ArcaAssetResolver {
    /// Base URL, no trailing slash. Example: `http://arca:3002`.
    pub base_url: String,
    /// Sent as `X-Subject-Id`. Required for non-public assets.
    pub subject_id: Option<String>,
    agent: ureq::Agent,
    pub max_body_bytes: usize,
}

impl ArcaAssetResolver {
    pub fn new(base_url: impl Into<String>, subject_id: Option<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            subject_id,
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(std::time::Duration::from_secs(10)))
                .build()
                .into(),
            max_body_bytes: 64 * 1024 * 1024,
        }
    }

    fn build_url(&self, uri: &str) -> Option<String> {
        let rest = uri.strip_prefix("arca://")?;
        if rest.is_empty() {
            return None;
        }
        // Pass through path verbatim — single id (`abc`) or id/derivative
        // (`abc/web-large`). Strip any query string; Arca's contract
        // doesn't use query params, derivatives are URL-path-encoded.
        let path = rest.split('?').next().unwrap_or(rest);
        Some(format!("{}/download/{}", self.base_url, path))
    }
}

impl AssetResolver for ArcaAssetResolver {
    fn fetch(&self, uri: &str) -> Result<Vec<u8>, AssetError> {
        if !uri.starts_with("arca://") {
            let scheme = uri.split("://").next().unwrap_or("?").to_string();
            return Err(AssetError::UnsupportedScheme { scheme });
        }
        let url = self
            .build_url(uri)
            .ok_or_else(|| AssetError::NotFound(uri.to_string()))?;
        let mut req = self.agent.get(&url);
        if let Some(sid) = &self.subject_id {
            req = req.header("X-Subject-Id", sid);
        }
        let mut resp = req
            .call()
            .map_err(|e| AssetError::Other(format!("Arca fetch {url}: {e}")))?;
        let mut buf: Vec<u8> = Vec::new();
        let mut reader = resp
            .body_mut()
            .as_reader()
            .take(self.max_body_bytes as u64 + 1);
        std::io::copy(&mut reader, &mut buf).map_err(|e| AssetError::Io {
            uri: uri.to_string(),
            source: e,
        })?;
        if buf.len() > self.max_body_bytes {
            return Err(AssetError::Other(format!(
                "Arca response exceeds max_body_bytes ({})",
                self.max_body_bytes
            )));
        }
        Ok(buf)
    }
}

/// Composes multiple resolvers; the first one whose `fetch` doesn't
/// return `UnsupportedScheme` wins.
pub struct CompositeAssetResolver {
    resolvers: Vec<Box<dyn AssetResolver>>,
}

impl CompositeAssetResolver {
    pub fn new() -> Self {
        Self {
            resolvers: Vec::new(),
        }
    }

    pub fn push(mut self, resolver: Box<dyn AssetResolver>) -> Self {
        self.resolvers.push(resolver);
        self
    }
}

impl Default for CompositeAssetResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetResolver for CompositeAssetResolver {
    fn fetch(&self, uri: &str) -> Result<Vec<u8>, AssetError> {
        let mut last_err: Option<AssetError> = None;
        for r in &self.resolvers {
            match r.fetch(uri) {
                Ok(b) => return Ok(b),
                Err(e @ AssetError::UnsupportedScheme { .. }) => {
                    last_err = Some(e);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap_or_else(|| AssetError::NotFound(uri.to_string())))
    }

    fn resolve_id(&self, id: &str) -> Option<String> {
        self.resolvers.iter().find_map(|r| r.resolve_id(id))
    }
}

/// Caches successful fetches by URI so repeated references to the same
/// asset hit the inner resolver only once.
///
/// **Optional byte cap**: when set via [`Self::with_max_bytes`], the
/// cache evicts least-recently-used entries to keep total cached body
/// size under the cap. A single entry larger than the cap is fetched
/// but not cached. Without a cap, the cache grows unbounded — fine for
/// short-lived single-doc renders, dangerous for long-running services.
///
/// **Errors are not cached** — transient failures retry on the next
/// call, and `AssetError::Io` isn't `Clone`-able anyway.
///
/// Thread-safe (`Send + Sync`); the cache is behind a `Mutex`.
pub struct CachingAssetResolver {
    inner: Box<dyn AssetResolver>,
    state: Mutex<CacheState>,
    max_bytes: Option<usize>,
}

struct CacheState {
    /// `(bytes, last_access_seq)`.
    entries: HashMap<String, (Arc<Vec<u8>>, u64)>,
    seq: u64,
    current_bytes: usize,
}

impl CachingAssetResolver {
    pub fn new(inner: Box<dyn AssetResolver>) -> Self {
        Self {
            inner,
            state: Mutex::new(CacheState {
                entries: HashMap::new(),
                seq: 0,
                current_bytes: 0,
            }),
            max_bytes: None,
        }
    }

    /// Limit total cached body size (in bytes). When inserting would
    /// exceed this, the least-recently-used entries are evicted until
    /// the new entry fits. Single entries larger than the cap are not
    /// cached but still served from the inner resolver.
    pub fn with_max_bytes(mut self, n: usize) -> Self {
        self.max_bytes = Some(n);
        self
    }

    /// Number of entries currently cached. Useful for instrumentation.
    pub fn cache_len(&self) -> usize {
        self.state.lock().map(|s| s.entries.len()).unwrap_or(0)
    }

    /// Total bytes currently held in the cache.
    pub fn cache_bytes(&self) -> usize {
        self.state.lock().map(|s| s.current_bytes).unwrap_or(0)
    }

    /// Drop all cached entries.
    pub fn clear(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.entries.clear();
            s.current_bytes = 0;
        }
    }
}

impl CacheState {
    fn next_seq(&mut self) -> u64 {
        self.seq = self.seq.wrapping_add(1);
        self.seq
    }

    /// Drop the LRU entry. Returns true if something was evicted.
    fn evict_one(&mut self) -> bool {
        let lru_key = self
            .entries
            .iter()
            .min_by_key(|(_, (_, ts))| *ts)
            .map(|(k, _)| k.clone());
        if let Some(key) = lru_key
            && let Some((bytes, _)) = self.entries.remove(&key)
        {
            self.current_bytes = self.current_bytes.saturating_sub(bytes.len());
            return true;
        }
        false
    }
}

impl AssetResolver for CachingAssetResolver {
    fn fetch(&self, uri: &str) -> Result<Vec<u8>, AssetError> {
        // Hot path: cache hit. Bump last-access while holding the lock.
        if let Ok(mut state) = self.state.lock()
            && let Some((bytes, _)) = state.entries.get(uri)
        {
            let copy = Vec::from(bytes.as_slice());
            let seq = state.next_seq();
            if let Some((_, ts)) = state.entries.get_mut(uri) {
                *ts = seq;
            }
            return Ok(copy);
        }
        // Miss: delegate, then store on success (subject to cap).
        let bytes = self.inner.fetch(uri)?;
        if let Ok(mut state) = self.state.lock() {
            let len = bytes.len();
            let fits_alone = self.max_bytes.is_none_or(|cap| len <= cap);
            if fits_alone {
                if let Some(cap) = self.max_bytes {
                    while state.current_bytes + len > cap {
                        if !state.evict_one() {
                            break; // empty cache; continue without cap
                        }
                    }
                }
                let arc = Arc::new(bytes.clone());
                let seq = state.next_seq();
                state.current_bytes += len;
                state.entries.insert(uri.to_string(), (arc, seq));
            }
            // else: response too big to cache; skip.
        }
        Ok(bytes)
    }

    fn resolve_id(&self, id: &str) -> Option<String> {
        self.inner.resolve_id(id)
    }
}

/// Sniff a binary blob's media format from magic bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
    Svg,
    Unknown,
}

pub fn sniff_format(data: &[u8]) -> MediaFormat {
    if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return MediaFormat::Png;
    }
    if data.starts_with(&[0xff, 0xd8, 0xff]) {
        return MediaFormat::Jpeg;
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return MediaFormat::Gif;
    }
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return MediaFormat::Webp;
    }
    // SVG sniff: look for `<svg` near the start, allowing for XML decl
    // or BOM. Cap at first 512 bytes to keep this cheap.
    let head = &data[..data.len().min(512)];
    if let Ok(text) = std::str::from_utf8(head)
        && text.contains("<svg")
    {
        return MediaFormat::Svg;
    }
    MediaFormat::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_resolver_always_errors() {
        let r = NullAssetResolver;
        assert!(matches!(
            r.fetch("anything").unwrap_err(),
            AssetError::NotFound(_)
        ));
    }

    #[test]
    fn fs_resolver_rejects_unsupported_scheme() {
        let r = FsAssetResolver::new("/tmp");
        let err = r.fetch("https://example.com/x.png").unwrap_err();
        assert!(matches!(err, AssetError::UnsupportedScheme { .. }));
    }

    #[test]
    fn fs_resolver_resolves_id_in_subdirectory() {
        use std::fs;
        let base = std::env::temp_dir().join("mdpdf-id-resolve-test");
        let _ = fs::remove_dir_all(&base);
        let nested = base.join("a/b");
        fs::create_dir_all(&nested).unwrap();
        let id = "0a02bb82-1a68-4c5c-883f-406361e1235e";
        fs::write(nested.join(format!("{id}.png")), b"\x89PNG-bytes").unwrap();

        let r = FsAssetResolver::new(&base);
        // The id resolves to a path (relative to the root) even though the
        // file is two levels down...
        let resolved = r.resolve_id(id).expect("id resolved from a sub-folder");
        assert!(resolved.ends_with(&format!("{id}.png")), "got {resolved}");
        // ...and that path fetches back through the same resolver.
        assert_eq!(r.fetch(&resolved).unwrap(), b"\x89PNG-bytes");
        // An unknown id yields None.
        assert!(r.resolve_id("no-such-id").is_none());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn sniff_png() {
        assert_eq!(
            sniff_format(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0]),
            MediaFormat::Png
        );
    }

    #[test]
    fn sniff_jpeg() {
        assert_eq!(sniff_format(&[0xff, 0xd8, 0xff, 0xe0]), MediaFormat::Jpeg);
    }

    #[test]
    fn sniff_svg() {
        assert_eq!(
            sniff_format(b"<?xml version=\"1.0\"?><svg xmlns=\"...\"/>"),
            MediaFormat::Svg
        );
        assert_eq!(sniff_format(b"<svg/>"), MediaFormat::Svg);
    }

    #[test]
    fn sniff_unknown() {
        assert_eq!(sniff_format(b"random bytes"), MediaFormat::Unknown);
    }

    #[test]
    fn http_resolver_rejects_non_http_scheme() {
        let r = HttpAssetResolver::new();
        let err = r.fetch("arca://abc").unwrap_err();
        assert!(matches!(err, AssetError::UnsupportedScheme { .. }));
        let err = r.fetch("file:///x").unwrap_err();
        assert!(matches!(err, AssetError::UnsupportedScheme { .. }));
    }

    #[test]
    fn arca_resolver_builds_download_url_for_bare_id() {
        let r = ArcaAssetResolver::new("http://arca:3002", None);
        assert_eq!(
            r.build_url("arca://abc-123").unwrap(),
            "http://arca:3002/download/abc-123"
        );
    }

    #[test]
    fn arca_resolver_builds_url_for_derivative() {
        let r = ArcaAssetResolver::new("http://arca:3002/", None);
        assert_eq!(
            r.build_url("arca://abc-123/web-large").unwrap(),
            "http://arca:3002/download/abc-123/web-large"
        );
    }

    #[test]
    fn arca_resolver_strips_trailing_slash_on_base() {
        let r = ArcaAssetResolver::new("http://arca:3002/", None);
        assert_eq!(
            r.build_url("arca://x").unwrap(),
            "http://arca:3002/download/x"
        );
    }

    #[test]
    fn arca_resolver_drops_query_params() {
        // The Arca contract uses path segments (derivative names), not
        // query strings — strip any `?...` suffix the markdoc author
        // may have added by mistake.
        let r = ArcaAssetResolver::new("http://arca:3002", None);
        assert_eq!(
            r.build_url("arca://abc?w=400").unwrap(),
            "http://arca:3002/download/abc"
        );
    }

    #[test]
    fn arca_resolver_rejects_non_arca_scheme() {
        let r = ArcaAssetResolver::new("http://arca:3002", None);
        let err = r.fetch("https://example.com/x.png").unwrap_err();
        assert!(matches!(err, AssetError::UnsupportedScheme { .. }));
    }

    /// Counts every fetch — used by the caching tests to verify the
    /// inner resolver only sees one call per unique URI.
    struct CountingResolver {
        calls: Mutex<Vec<String>>,
    }

    impl CountingResolver {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
            }
        }
        fn count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl AssetResolver for CountingResolver {
        fn fetch(&self, uri: &str) -> Result<Vec<u8>, AssetError> {
            self.calls.lock().unwrap().push(uri.to_string());
            // Distinct payload per URI so we can verify cache returns
            // the right one.
            Ok(format!("body-of-{uri}").into_bytes())
        }
    }

    /// Newtype that delegates to a shared inner so the test can keep a
    /// reference to the inner counter.
    struct DelegateResolver(Arc<CountingResolver>);
    impl AssetResolver for DelegateResolver {
        fn fetch(&self, uri: &str) -> Result<Vec<u8>, AssetError> {
            self.0.fetch(uri)
        }
    }

    #[test]
    fn caching_resolver_serves_repeated_uris_from_cache() {
        let counter = Arc::new(CountingResolver::new());
        let cache = CachingAssetResolver::new(Box::new(DelegateResolver(counter.clone())));

        let a1 = cache.fetch("arca://abc").unwrap();
        let a2 = cache.fetch("arca://abc").unwrap();
        let b = cache.fetch("arca://other").unwrap();
        let a3 = cache.fetch("arca://abc").unwrap();

        assert_eq!(a1, b"body-of-arca://abc");
        assert_eq!(a2, a1);
        assert_eq!(a3, a1);
        assert_eq!(b, b"body-of-arca://other");

        // Inner saw exactly two unique URIs, regardless of repeats.
        assert_eq!(counter.count(), 2);
        assert_eq!(cache.cache_len(), 2);
    }

    #[test]
    fn caching_resolver_does_not_cache_errors() {
        // Inner that errors then succeeds on second call for the same URI.
        struct FlakyResolver {
            calls: Mutex<usize>,
        }
        impl AssetResolver for FlakyResolver {
            fn fetch(&self, _uri: &str) -> Result<Vec<u8>, AssetError> {
                let mut c = self.calls.lock().unwrap();
                *c += 1;
                if *c == 1 {
                    Err(AssetError::Other("first call fails".into()))
                } else {
                    Ok(b"ok".to_vec())
                }
            }
        }
        let cache = CachingAssetResolver::new(Box::new(FlakyResolver {
            calls: Mutex::new(0),
        }));
        assert!(cache.fetch("x").is_err()); // first fails
        assert_eq!(cache.fetch("x").unwrap(), b"ok"); // retries → succeeds
        assert_eq!(cache.fetch("x").unwrap(), b"ok"); // now cached
    }

    /// Resolver returning a fixed-size payload per URI so we can
    /// reason about the cache's byte accounting precisely.
    struct FixedSizeResolver {
        payload_bytes: usize,
    }
    impl AssetResolver for FixedSizeResolver {
        fn fetch(&self, _uri: &str) -> Result<Vec<u8>, AssetError> {
            Ok(vec![0u8; self.payload_bytes])
        }
    }

    #[test]
    fn caching_resolver_evicts_when_over_max_bytes() {
        // Cap = 100 bytes; each fetch returns 40 bytes. Caching three
        // distinct URIs (3 × 40 = 120 > 100) should evict the LRU.
        let inner = FixedSizeResolver { payload_bytes: 40 };
        let cache = CachingAssetResolver::new(Box::new(inner)).with_max_bytes(100);

        cache.fetch("a").unwrap();
        cache.fetch("b").unwrap();
        assert_eq!(cache.cache_len(), 2);
        assert_eq!(cache.cache_bytes(), 80);

        // Touch "a" so it's most-recent; "b" becomes LRU.
        cache.fetch("a").unwrap();

        // Insert "c" → must evict "b" (LRU) to fit.
        cache.fetch("c").unwrap();
        assert_eq!(cache.cache_len(), 2);
        assert_eq!(cache.cache_bytes(), 80);
    }

    #[test]
    fn caching_resolver_skips_caching_oversized_entries() {
        // Cap = 50 bytes; payload = 100 bytes per fetch — too big to cache.
        let counter = Arc::new(CountingResolver::new());
        // Wrap in a small resolver that returns bigger bodies.
        struct Big(Arc<CountingResolver>);
        impl AssetResolver for Big {
            fn fetch(&self, uri: &str) -> Result<Vec<u8>, AssetError> {
                self.0.fetch(uri)?; // bump counter
                Ok(vec![0u8; 100])
            }
        }
        let cache = CachingAssetResolver::new(Box::new(Big(counter.clone()))).with_max_bytes(50);

        cache.fetch("x").unwrap();
        cache.fetch("x").unwrap();
        // Both fetches went to the inner — the body was too big to cache.
        assert_eq!(counter.count(), 2);
        assert_eq!(cache.cache_len(), 0);
    }

    #[test]
    fn caching_resolver_clear_drops_entries() {
        let counter = Arc::new(CountingResolver::new());
        let cache = CachingAssetResolver::new(Box::new(DelegateResolver(counter.clone())));
        cache.fetch("x").unwrap();
        assert_eq!(cache.cache_len(), 1);
        cache.clear();
        assert_eq!(cache.cache_len(), 0);
        cache.fetch("x").unwrap();
        // Inner saw two calls because we cleared between.
        assert_eq!(counter.count(), 2);
    }

    #[test]
    fn composite_skips_resolvers_that_dont_support_scheme() {
        // FS handles only fs paths (and `file://`); Composite should
        // try it, see UnsupportedScheme for `arca://`, then move on.
        // With nothing else able to handle it, the final error is the
        // last UnsupportedScheme observed.
        let r = CompositeAssetResolver::new().push(Box::new(FsAssetResolver::new("/tmp")));
        let err = r.fetch("arca://x").unwrap_err();
        assert!(matches!(err, AssetError::UnsupportedScheme { .. }));
    }
}
