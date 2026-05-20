//! Content-hashed static asset manifest. In production, walks a directory
//! at startup and computes sha256(file)[..8] for each file. The
//! `resolve` function returns the hashed URL for a logical path; the
//! `resolve_hashed_request` function recognizes the hashed pattern when serving.
//!
//! In dev, `Assets::dev()` returns a no-op manifest so `resolve` returns
//! the raw logical path. Tailwind --watch can edit `app.css` freely.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

#[derive(Clone)]
pub enum Assets {
    /// Production: walk the static dir, compute hashes, return hashed URLs.
    Hashed {
        root: PathBuf,
        manifest: HashMap<String, String>, // "css/app.css" -> "css/app.<hash>.css"
    },
    /// Dev: identity mapping, raw paths.
    Passthrough { root: PathBuf },
}

#[derive(Debug, thiserror::Error)]
pub enum AssetsError {
    #[error("read static dir: {0}")]
    Read(#[from] std::io::Error),
}

impl Assets {
    pub fn production(root: PathBuf) -> Result<Self, AssetsError> {
        let mut manifest = HashMap::new();
        walk_files(&root, &root, &mut manifest)?;
        Ok(Self::Hashed { root, manifest })
    }

    pub fn dev(root: PathBuf) -> Self {
        Self::Passthrough { root }
    }

    pub fn root(&self) -> &Path {
        match self {
            Self::Hashed { root, .. } => root,
            Self::Passthrough { root } => root,
        }
    }

    /// Resolve a logical path (e.g. "css/app.css") to a URL path (without
    /// the leading "/static/" prefix — callers prepend that).
    pub fn resolve(&self, logical: &str) -> String {
        match self {
            Self::Hashed { manifest, .. } => manifest
                .get(logical)
                .cloned()
                .unwrap_or_else(|| logical.to_owned()),
            Self::Passthrough { .. } => logical.to_owned(),
        }
    }

    /// If `url_path` matches `<dir>/<name>.<8hex>.<ext>` AND the manifest
    /// has the same hash for the unhashed path, return the on-disk path
    /// of the unhashed file. Otherwise return None.
    pub fn resolve_hashed_request(&self, url_path: &str) -> Option<PathBuf> {
        let Self::Hashed { root, manifest } = self else {
            return None;
        };
        // Find the unhashed entry whose hashed value equals url_path.
        let unhashed = manifest
            .iter()
            .find_map(|(unhashed, hashed)| (hashed == url_path).then(|| unhashed.clone()))?;
        Some(root.join(&unhashed))
    }
}

fn walk_files(
    root: &Path,
    dir: &Path,
    manifest: &mut HashMap<String, String>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_files(root, &path, manifest)?;
            continue;
        }
        // Skip precompressed siblings; they share the unhashed path's hash.
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.ends_with(".gz") || name.ends_with(".br") {
            continue;
        }
        let rel = path.strip_prefix(root).unwrap();
        let logical = rel.to_string_lossy().replace('\\', "/");
        let hash = hash_file(&path)?;
        let hashed = inject_hash(&logical, &hash);
        manifest.insert(logical, hashed);
    }
    Ok(())
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    // First 8 hex chars is plenty for cache busting; collision risk is
    // negligible at the file counts we care about.
    Ok(hex_short(&digest[..]))
}

fn hex_short(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(8);
    for b in &bytes[..4] {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// "css/app.css" + "abcd1234" -> "css/app.abcd1234.css"
fn inject_hash(logical: &str, hash: &str) -> String {
    if let Some(dot) = logical.rfind('.') {
        let (stem, ext) = logical.split_at(dot);
        format!("{stem}.{hash}{ext}")
    } else {
        format!("{logical}.{hash}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(p: &Path, body: &[u8]) {
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, body).unwrap();
    }

    #[test]
    fn dev_passthrough_returns_logical_path() {
        let dir = tempfile::tempdir().unwrap();
        let a = Assets::dev(dir.path().to_path_buf());
        assert_eq!(a.resolve("css/app.css"), "css/app.css");
    }

    #[test]
    fn production_injects_hash_before_extension() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("css/app.css"), b"body { color: red; }");
        let a = Assets::production(dir.path().to_path_buf()).unwrap();
        let resolved = a.resolve("css/app.css");
        assert!(resolved.starts_with("css/app."));
        assert!(resolved.ends_with(".css"));
        // The injected hash has 8 hex chars + 1 dot before the extension.
        let middle = &resolved["css/app.".len()..resolved.len() - ".css".len()];
        assert_eq!(middle.len(), 8);
        assert!(middle.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn resolve_hashed_request_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("css/app.css"), b"hello");
        let a = Assets::production(dir.path().to_path_buf()).unwrap();
        let resolved = a.resolve("css/app.css");
        let on_disk = a.resolve_hashed_request(&resolved).unwrap();
        assert!(on_disk.ends_with("css/app.css"));
    }

    #[test]
    fn unknown_logical_path_returns_itself() {
        let dir = tempfile::tempdir().unwrap();
        let a = Assets::production(dir.path().to_path_buf()).unwrap();
        assert_eq!(a.resolve("not/in/manifest.png"), "not/in/manifest.png");
    }

    #[test]
    fn precompressed_siblings_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("js/app.js"), b"console.log('hi');");
        write(&dir.path().join("js/app.js.gz"), b"compressed");
        write(&dir.path().join("js/app.js.br"), b"compressed");
        let a = Assets::production(dir.path().to_path_buf()).unwrap();
        let Assets::Hashed { manifest, .. } = &a else {
            panic!()
        };
        assert!(manifest.contains_key("js/app.js"));
        assert!(!manifest.contains_key("js/app.js.gz"));
        assert!(!manifest.contains_key("js/app.js.br"));
    }
}
