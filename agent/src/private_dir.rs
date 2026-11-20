//! Creates a private directory chain resilient to a restrictive `umask`.
//!
//! `std::fs::create_dir_all` applies the process umask to every directory it
//! creates. With `umask 0777` the intermediate directories become mode `0000`,
//! so a later `chmod` on the leaf alone cannot traverse the chain. On Unix we
//! therefore set `0700` on every component we create.
//!
//! Permission policy:
//! - pre-existing ancestors are left untouched (we never widen somebody
//!   else's permissions);
//! - the leaf is always normalised to `0700`, even when it already existed
//!   with a looser mode;
//! - a component a concurrent starter created between our `exists()` check and
//!   `create_dir` (EEXIST) is normalised to `0700` too, after confirming it is
//!   a directory.
//!
//! A regular file at a path we are about to chmod is refused with
//! `InvalidInput`; its permissions and contents are never modified.

use std::io;
use std::path::Path;

/// Creates `dir` (and missing parents) with 0700 permissions on Unix.
///
/// A regular file at `dir` or at an EEXIST component is never chmodded; an
/// `InvalidInput` error is returned instead.
pub(crate) fn ensure_private_dir_chain(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        fn chmod_private_dir(path: &Path) -> io::Result<()> {
            if !path.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "private directory path is not a directory",
                ));
            }
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        }

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
            // converges on the same chain. `chmod_private_dir` verifies the
            // result is a directory before touching its permissions.
            if let Err(err) = std::fs::create_dir(&component) {
                if err.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(err);
                }
                std::fs::create_dir_all(&component)?;
            }
            chmod_private_dir(&component)?;
        }
        // Always normalise the leaf: it may pre-exist with a looser mode. A
        // regular file at `dir` is refused instead of being chmodded.
        chmod_private_dir(dir)?;
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
    use std::path::PathBuf;
    use std::sync::{Arc, Barrier};

    const CHILD_FLAG: &str = "TM_PRIVDIR_UMASK_CHILD";
    const ROOT_VAR: &str = "TM_PRIVDIR_UMASK_ROOT";
    const TEST_FILTER: &str = "private_dir_chain_survives_a_restrictive_umask";

    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path).expect("stat").permissions().mode() & 0o777
    }

    fn fresh_root(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{}-{}-{}",
            prefix,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn private_dir_chain_survives_a_restrictive_umask() {
        if std::env::var_os(CHILD_FLAG).is_some() {
            // Child: the parent launched us through `sh` with umask 0777.
            let root = PathBuf::from(std::env::var_os(ROOT_VAR).expect("root env"));

            // Control: prove the umask really is 0777 in this process. A bare
            // create_dir must produce mode 0000 here. Restore 0700 afterwards
            // so the parent can remove the tree.
            let control = root.join("control");
            std::fs::create_dir(&control).expect("control dir");
            assert_eq!(mode_of(&control), 0o000, "child must run under umask 0777");
            std::fs::set_permissions(&control, std::fs::Permissions::from_mode(0o700))
                .expect("restore control mode");

            ensure_private_dir_chain(&root.join("a/b/c")).expect("ensure chain");
            return;
        }

        // Parent: relaunch this same test binary under a shell that installs
        // umask 0777. No external coreutils and no unsafe pre_exec needed.
        let root = fresh_root("tm-private-dir-umask");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .expect("chmod root");

        let exe = std::env::current_exe().expect("test binary");
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg("umask 0777; exec \"$0\" --nocapture \"$1\"")
            .arg(&exe)
            .arg(TEST_FILTER)
            .env(CHILD_FLAG, "1")
            .env(ROOT_VAR, &root)
            .status()
            .expect("spawn umask child");
        assert!(status.success(), "umask 0777 child test failed");

        for component in [root.join("a"), root.join("a/b"), root.join("a/b/c")] {
            assert_eq!(
                mode_of(&component),
                0o700,
                "component {:?} must converge to 0700 under umask 0777",
                component
            );
        }
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn concurrent_starters_converge_on_one_private_chain() {
        // Two threads start the identical chain behind a barrier. Whichever
        // wins a `create_dir` pushes the other through the EEXIST ->
        // `create_dir_all` branch; both calls must still return Ok and observe
        // a fully 0700 chain. Repeated fresh roots make the race deterministic
        // in practice without touching production code with test hooks.
        const ROUNDS: usize = 16;
        for round in 0..ROUNDS {
            let root = fresh_root(&format!("tm-privdir-concurrent-{round}"));
            std::fs::create_dir_all(&root).expect("create root");
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
                .expect("chmod root");
            let target = root.join(format!("r{round}/a/b/c/d/e"));

            let gate = Arc::new(Barrier::new(2));
            let left_target = target.clone();
            let left_gate = Arc::clone(&gate);
            let left = std::thread::spawn(move || {
                left_gate.wait();
                ensure_private_dir_chain(&left_target)
            });
            let right_target = target.clone();
            let right = std::thread::spawn(move || {
                gate.wait();
                std::thread::yield_now();
                ensure_private_dir_chain(&right_target)
            });
            left.join().expect("left starter").expect("left ensure");
            right.join().expect("right starter").expect("right ensure");

            let mut cursor = target.as_path();
            while cursor != root.as_path() {
                assert_eq!(
                    mode_of(cursor),
                    0o700,
                    "component {cursor:?} must converge to 0700 after the race"
                );
                cursor = cursor.parent().expect("root is the top ancestor");
            }
            std::fs::remove_dir_all(&root).expect("cleanup");
        }
    }

    #[test]
    fn existing_0755_leaf_is_normalised_but_ancestors_are_untouched() {
        let root = fresh_root("tm-privdir-modes");
        let ancestor = root.join("keep-0755");
        let leaf = ancestor.join("leaf");
        std::fs::create_dir_all(&leaf).expect("create chain");
        let paths = [&root, &ancestor, &leaf];
        for path in paths {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod 0755");
        }
        for path in paths {
            assert_eq!(mode_of(path), 0o755, "precondition for {path:?}");
        }

        ensure_private_dir_chain(&leaf).expect("normalise existing leaf");

        assert_eq!(
            mode_of(&root),
            0o755,
            "pre-existing ancestor must stay 0755"
        );
        assert_eq!(
            mode_of(&ancestor),
            0o755,
            "pre-existing ancestor must stay 0755"
        );
        assert_eq!(
            mode_of(&leaf),
            0o700,
            "existing leaf must be normalised to 0700"
        );

        // A chain created below that ancestor follows the opposite rule: only
        // the new components get 0700, the pre-existing parent is not touched.
        let created = ancestor.join("new/a");
        ensure_private_dir_chain(&created).expect("create below existing ancestor");
        assert_eq!(mode_of(&ancestor), 0o755, "ancestor must stay 0755");
        assert_eq!(mode_of(&ancestor.join("new")), 0o700);
        assert_eq!(mode_of(&created), 0o700);

        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn existing_regular_file_is_never_chmodded() {
        let root = fresh_root("tm-private-dir-file");
        std::fs::create_dir_all(&root).expect("create root");
        let file = root.join("leaf");
        std::fs::write(&file, b"keep").expect("write file");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))
            .expect("chmod file");

        let err = ensure_private_dir_chain(&file).expect_err("a file must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            mode_of(&file),
            0o755,
            "existing file mode must stay unchanged"
        );
        assert_eq!(std::fs::read(&file).expect("read"), b"keep");

        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
