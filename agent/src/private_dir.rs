//! Creates a private directory chain resilient to a restrictive `umask`.
//!
//! `std::fs::create_dir_all` applies the process umask to every directory it
//! creates. With `umask 0777` the intermediate directories become mode `0000`,
//! so a later `chmod` on the leaf alone cannot traverse the chain. On Unix we
//! therefore set `0700` on every component we create (and on the leaf), while
//! leaving pre-existing directories untouched so we never widen somebody
//! else's permissions.

use std::io;
use std::path::Path;

/// Creates `dir` (and missing parents) with 0700 permissions on Unix.
pub(crate) fn ensure_private_dir_chain(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        // Collect missing ancestors from the top down. `create_dir` on each
        // component after the first would otherwise inherit a restrictive
        // umask; setting 0700 immediately keeps the chain traversable.
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
        // Always normalise the leaf: it may pre-exist with a looser mode.
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
    fn private_dir_chain_survives_a_restrictive_umask() {
        let root = std::env::temp_dir().join(format!(
            "tm-private-dir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let target = root.join("a/b/c");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .expect("chmod root");
        ensure_private_dir_chain(&target).expect("ensure chain");
        for component in [root.join("a"), root.join("a/b"), target.clone()] {
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
