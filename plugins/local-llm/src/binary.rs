use std::path::{Path, PathBuf};

use crate::UnavailableReason;

/// The bundled server's name. Not `librelog-sidecar-*`: debug builds pass
/// those through a legacy shim, and the app's exit sweep kills every
/// process whose name contains it.
pub const SERVER_NAME: &str = "librelog-llama-server";

/// Debug builds only: run this llama-server instead of the bundled one, on
/// any platform.
pub const SERVER_OVERRIDE_ENV: &str = "LIBRELOG_LLAMA_SERVER";

/// Built-in models ship for Apple silicon outside the App Store and for
/// Linux on x86-64.
const SHIPPED: bool = cfg!(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64")
));

pub fn resolve() -> Result<PathBuf, UnavailableReason> {
    if cfg!(debug_assertions)
        && let Some(path) = std::env::var_os(SERVER_OVERRIDE_ENV).map(PathBuf::from)
        && path.is_file()
    {
        return Ok(path);
    }

    if !SHIPPED {
        return Err(UnavailableReason::UnsupportedPlatform);
    }
    if !cpu_supported() {
        return Err(UnavailableReason::UnsupportedCpu);
    }

    let exe = std::env::current_exe().ok();
    let prepared = cfg!(debug_assertions).then(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/desktop/src-tauri/binaries")
            .join(format!("{SERVER_NAME}-{}", env!("LOCAL_LLM_TARGET_TRIPLE")))
    });
    locate(exe.as_deref(), prepared.as_deref()).ok_or(UnavailableReason::MissingBinary)
}

/// Tauri puts external binaries next to the app's executable, without the
/// target triple: in `Contents/MacOS` on macOS, `/usr/bin` in the RPM and
/// deb, and `usr/bin` inside the AppImage's mount. A debug build falls back
/// to the server `cargo xtask prepare-binaries` left in `binaries/`.
fn locate(exe: Option<&Path>, prepared: Option<&Path>) -> Option<PathBuf> {
    exe.and_then(Path::parent)
        .map(|dir| dir.join(SERVER_NAME))
        .filter(|path| path.is_file())
        .or_else(|| {
            prepared
                .filter(|path| path.is_file())
                .map(Path::to_path_buf)
        })
}

/// The Linux server is built for x86-64 with AVX2, FMA, F16C and BMI2
/// (`GGML_NATIVE=OFF`), like transcription; an older CPU would crash on an
/// illegal instruction instead of failing.
fn cpu_supported() -> bool {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        std::arch::is_x86_feature_detected!("avx")
            && std::arch::is_x86_feature_detected!("avx2")
            && std::arch::is_x86_feature_detected!("fma")
            && std::arch::is_x86_feature_detected!("f16c")
            && std::arch::is_x86_feature_detected!("bmi2")
    }
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // An RPM, deb or AppImage install must find the server beside the app,
    // and a dev build without it must say it is missing.
    #[test]
    fn server_is_found_beside_the_app_or_in_binaries() {
        let root = tempfile::tempdir().unwrap();
        let installed = root.path().join("usr/bin");
        let dev = root.path().join("target/debug");
        let binaries = root.path().join("binaries");
        for dir in [&installed, &dev, &binaries] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(installed.join(SERVER_NAME), "").unwrap();
        let prepared = binaries.join(format!("{SERVER_NAME}-x86_64-unknown-linux-gnu"));

        let app = installed.join("librelog");
        assert_eq!(
            locate(Some(&app), Some(&prepared)),
            Some(installed.join(SERVER_NAME))
        );

        let dev_app = dev.join("librelog-dev");
        assert_eq!(locate(Some(&dev_app), Some(&prepared)), None);
        assert_eq!(locate(Some(&dev_app), None), None);

        std::fs::write(&prepared, "").unwrap();
        assert_eq!(locate(Some(&dev_app), Some(&prepared)), Some(prepared));
    }
}
