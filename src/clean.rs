//! `--clean`: delete the build caches (spec §8).

use crate::config::Config;
use crate::tools::find_in_path;
use std::path::Path;
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
    let target = remove_cache_dir("cargo target dir", &cfg.cargo_target_dir, &cfg.home);
    let sccache = remove_cache_dir("sccache cache", &cfg.sccache_dir, &cfg.home);
    let ok = target && sccache;
    if ok {
        println!("Build caches cleaned.");
    }
    ok
}

/// Remove `dir` unless it resolves to `/` or `home`. True on success (including nothing to remove).
pub fn remove_cache_dir(label: &str, dir: &Path, home: &Path) -> bool {
    if !dir.exists() {
        println!("No {label} at {}; nothing to remove.", dir.display());
        return true;
    }
    let Ok(resolved) = dir.canonicalize() else {
        eprintln!("Error: can't resolve {label} at {}.", dir.display());
        return false;
    };
    let home = home.canonicalize().unwrap_or_else(|_| home.to_owned());
    if resolved == Path::new("/") || resolved == home {
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
        assert!(!remove_cache_dir("cache", Path::new("/"), &home));
        assert!(!remove_cache_dir("cache", &home, &home));
        assert!(home.exists());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn missing_is_fine_and_existing_is_removed() {
        let home = scratch("home2");
        let cache = home.join("cache");
        assert!(remove_cache_dir("cache", &cache, &home));
        std::fs::create_dir_all(cache.join("sub")).unwrap();
        assert!(remove_cache_dir("cache", &cache, &home));
        assert!(!cache.exists());
        std::fs::remove_dir_all(home).unwrap();
    }
}
