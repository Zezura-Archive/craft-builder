//! `--clean`: delete the build caches (spec §8).

use crate::config::Config;
use crate::tools::find_in_path;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Stop sccache, then remove the cargo target dir and sccache dir. True if both succeeded.
pub fn run(cfg: &Config) -> bool {
    if find_in_path("sccache").is_some() {
        let _ = Command::new("sccache")
            .arg("--stop-server")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let (dest, build, target_dir) = (&cfg.dest_dir, &cfg.build_dir, &cfg.cargo_target_dir);
    let target = remove_cache_dir("cargo target dir", target_dir, &cfg.home, &[dest, build]);
    let sccache = remove_cache_dir(
        "sccache cache",
        &cfg.sccache_dir,
        &cfg.home,
        &[dest, build, target_dir],
    );
    let ok = target && sccache;
    if ok {
        println!("Build caches cleaned.");
    }
    ok
}

/// `path` and, if it exists, its canonical form.
fn variants(path: &Path) -> Vec<PathBuf> {
    let mut all = vec![path.to_owned()];
    all.extend(path.canonicalize().ok().filter(|c| c != path));
    all
}

/// Whether deleting `path` is unsafe: it is `/`, `home` or an ancestor of it, or equals
/// or contains any `keep` path. Compared both lexically and canonicalized.
pub fn is_protected(path: &Path, home: &Path, keep: &[&Path]) -> bool {
    let protected: Vec<PathBuf> = std::iter::once(home)
        .chain(keep.iter().copied())
        .flat_map(variants)
        .collect();
    variants(path).iter().any(|p| {
        p == Path::new("/")
            || (variants(home).iter().any(|h| h.starts_with(p)))
            || protected.iter().any(|k| k.starts_with(p))
    })
}

/// Remove `dir` unless it is protected (see [`is_protected`]). True on success (including nothing to remove).
pub fn remove_cache_dir(label: &str, dir: &Path, home: &Path, keep: &[&Path]) -> bool {
    if !dir.exists() {
        println!("No {label} at {}; nothing to remove.", dir.display());
        return true;
    }
    let Ok(resolved) = dir.canonicalize() else {
        eprintln!("Error: can't resolve {label} at {}.", dir.display());
        return false;
    };
    if is_protected(dir, home, keep) {
        eprintln!(
            "Error: refusing to remove {label} at {}.",
            resolved.display()
        );
        return false;
    }
    println!("Removing {label}: {}", resolved.display());
    let removed = if resolved.is_dir() {
        std::fs::remove_dir_all(&resolved)
    } else {
        std::fs::remove_file(&resolved)
    };
    if let Err(err) = removed {
        eprintln!("Error: failed to remove {}: {err}", resolved.display());
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("craft-builder-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn refuses_root_and_home() {
        let home = scratch("home");
        assert!(!remove_cache_dir("cache", Path::new("/"), &home, &[]));
        assert!(!remove_cache_dir("cache", &home, &home, &[]));
        assert!(home.exists());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn missing_is_fine_and_existing_is_removed() {
        let home = scratch("home2");
        let cache = home.join("cache");
        assert!(remove_cache_dir("cache", &cache, &home, &[]));
        std::fs::create_dir_all(cache.join("sub")).unwrap();
        assert!(remove_cache_dir("cache", &cache, &home, &[]));
        assert!(!cache.exists());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn protects_home_ancestors_and_kept_paths() {
        let home = Path::new("/h/u");
        let keep = [Path::new("/h/u/repos/forks"), Path::new("/b/builds")];
        for p in [
            "/",
            "/h",
            "/h/u",
            "/h/u/repos",
            "/h/u/repos/forks",
            "/b",
            "/b/builds",
        ] {
            assert!(is_protected(Path::new(p), home, &keep), "{p}");
        }
        for p in ["/h/u/cache", "/h/u/repos/forks/x", "/b/other", "/h/user"] {
            assert!(!is_protected(Path::new(p), home, &keep), "{p}");
        }
    }

    #[test]
    fn protects_through_symlinks() {
        let root = scratch("link");
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&home, &link).unwrap();
        assert!(is_protected(&link, Path::new("/nowhere"), &[&home]));
        assert!(!remove_cache_dir("cache", &link, &home, &[]));
        std::fs::remove_dir_all(root).unwrap();
    }
}
