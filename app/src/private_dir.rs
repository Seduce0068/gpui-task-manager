//! Creates a private directory chain resilient to a restrictive `umask`.
//!
//! Mirrors `agent/src/private_dir.rs`: `create_dir_all` would apply `umask
//! 0777` to every component, leaving an untraversable `0000` chain. On Unix we
//! set `0700` on every component we create, without widening pre-existing
//! directories.

use std::io;
use std::path::Path;

/// Creates `dir` (and missing parents) with 0700 permissions on Unix.
pub(crate) fn ensure_private_dir_chain(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let mut missing = Vec::new();
        let mut cursor = dir;
        while !cursor.exists() {
            match cursor.parent() {
                Some(parent) => {
                    missing.push(cursor.to_path_buf());
                    cursor = parent;
                }
                None => break,
            }
        }
        for component in missing.into_iter().rev() {
            // A concurrent starter may have created this component between our
            // `exists()` check and `create_dir`; EEXIST is fine, create_all
            // converges on the same chain.
            if let Err(err) = std::fs::create_dir(&component) {
                if err.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(err);
                }
                std::fs::create_dir_all(&component)?;
            }
            std::fs::set_permissions(&component, std::fs::Permissions::from_mode(0o700))?;
        }
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    #[test]
    fn private_dir_chain_sets_every_component_to_0700() {
        let root = std::env::temp_dir().join(format!(
            "tm-app-private-dir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let target = root.join("x/y/z");
        ensure_private_dir_chain(&target).expect("ensure chain");
        for component in [
            root.clone(),
            root.join("x"),
            root.join("x/y"),
            target.clone(),
        ] {
            let mode = std::fs::metadata(&component)
                .expect("stat")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o700, "component {:?} must be private", component);
        }
        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
