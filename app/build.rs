//! Adds the local `native-libs` shim directory to the linker search path.
//!
//! Kubuntu ships `libxkbcommon-x11.so.0` at runtime but not the unversioned
//! `libxkbcommon-x11.so` development symlink. GPUI's X11 feature links against
//! it, so on hosts without `libxkbcommon-x11-dev` the final link fails. The
//! repository ships `scripts/dev-setup.sh`, which creates a local symlink under
//! `app/native-libs/`; this build script points the linker at that directory.
//! No system files are modified and no root privileges are required.

use std::path::PathBuf;

fn main() {
    let native_libs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("native-libs");
    if native_libs.is_dir() {
        println!("cargo:rustc-link-search=native={}", native_libs.display());
        println!("cargo:rerun-if-changed={}", native_libs.display());
    }
    println!("cargo:rerun-if-changed=build.rs");
}
