//! Builds the llama-server the app bundles as `librelog-llama-server`: one
//! statically linked executable from llama.cpp at a pinned commit, with the
//! web UI, RPC, HTTPS and subprocess support left out. On macOS it embeds the
//! Metal shaders; on Linux x86-64 it carries the Vulkan backend, links only
//! glibc and the system's Vulkan loader, and runs on the CPU when no Vulkan
//! GPU is usable.

use anyhow::{Context, Result, bail, ensure};
use std::{fs, path::Path};
use xshell::{Shell, cmd};

const REPOSITORY: &str = "https://github.com/ggml-org/llama.cpp";
/// llama.cpp v0.9.0.
const TAG: &str = "b12110";
const COMMIT: &str = "cd959bae99ff4bed22bef2604f45241c09bbb8c8";

/// Built-in models ship for Apple silicon outside the App Store and for Linux
/// on x86-64.
fn configure(triple: &str) -> Option<&'static [&'static str]> {
    match triple {
        "aarch64-apple-darwin" => Some(MACOS_CONFIGURE),
        "x86_64-unknown-linux-gnu" => Some(LINUX_X86_64_CONFIGURE),
        _ => None,
    }
}

const MACOS_CONFIGURE: &[&str] = &[
    "-DCMAKE_BUILD_TYPE=Release",
    "-DBUILD_SHARED_LIBS=OFF",
    "-DLLAMA_BUILD_SERVER=ON",
    "-DLLAMA_BUILD_TOOLS=ON",
    "-DLLAMA_BUILD_TESTS=OFF",
    "-DLLAMA_BUILD_EXAMPLES=OFF",
    "-DLLAMA_BUILD_APP=OFF",
    "-DGGML_METAL_EMBED_LIBRARY=ON",
    // The build machine's CPU features would crash older Apple silicon.
    "-DGGML_NATIVE=OFF",
    "-DGGML_RPC=OFF",
    "-DLLAMA_CONNECT=OFF",
    "-DLLAMA_OPENSSL=OFF",
    "-DLLAMA_BUILD_UI=OFF",
    "-DLLAMA_USE_PREBUILT_UI=OFF",
    // Without subprocess support the server cannot run tools at all.
    "-DLLAMA_SUBPROCESS=OFF",
    "-DCMAKE_OSX_DEPLOYMENT_TARGET=15.0",
    "-DCMAKE_OSX_ARCHITECTURES=arm64",
];

const LINUX_X86_64_CONFIGURE: &[&str] = &[
    "-DCMAKE_BUILD_TYPE=Release",
    "-DBUILD_SHARED_LIBS=OFF",
    "-DLLAMA_BUILD_SERVER=ON",
    "-DLLAMA_BUILD_TOOLS=ON",
    "-DLLAMA_BUILD_TESTS=OFF",
    "-DLLAMA_BUILD_EXAMPLES=OFF",
    "-DLLAMA_BUILD_APP=OFF",
    "-DGGML_VULKAN=ON",
    // AVX2, FMA, F16C and BMI2, the floor transcription already requires;
    // the plugin refuses CPUs below it instead of crashing.
    "-DGGML_NATIVE=OFF",
    // ggml's own thread pool, so libgomp need not be installed.
    "-DGGML_OPENMP=OFF",
    "-DGGML_RPC=OFF",
    "-DLLAMA_CONNECT=OFF",
    "-DLLAMA_OPENSSL=OFF",
    "-DLLAMA_BUILD_UI=OFF",
    "-DLLAMA_USE_PREBUILT_UI=OFF",
    "-DLLAMA_SUBPROCESS=OFF",
    "-DCMAKE_EXE_LINKER_FLAGS=-static-libstdc++ -static-libgcc -s",
];

pub(crate) fn prepare(
    sh: &Shell,
    triple: &str,
    work_dir: &Path,
    binaries_dir: &Path,
) -> Result<()> {
    let Some(configure) = configure(triple) else {
        println!(
            "prepare-binaries: skipping librelog-llama-server: built-in models do not ship for {triple}"
        );
        return Ok(());
    };

    let source = work_dir.join(format!("llama.cpp-{TAG}"));
    let build = source.join("build-librelog");
    let dst = binaries_dir.join(format!("librelog-llama-server-{triple}"));
    let stamp = build.join("librelog-stamp");
    let expected_stamp = format!("{COMMIT}\n{}\n", configure.join("\n"));
    if dst.is_file() && fs::read_to_string(&stamp).ok().as_deref() == Some(&expected_stamp) {
        println!("prepare-binaries: binaries/librelog-llama-server-{triple} is up to date");
        return Ok(());
    }

    // A CI cache can bring back the directory without a working checkout;
    // git then answers for the repository around it. Start over from a
    // fresh clone whenever the checkout is not the pinned one.
    if source.exists() && checkout_head(sh, &source).as_deref() != Some(COMMIT) {
        fs::remove_dir_all(&source).context("remove a stale llama.cpp checkout")?;
    }
    if !source.exists() {
        fs::create_dir_all(work_dir).context("create llama.cpp work directory")?;
        cmd!(
            sh,
            "git clone --depth 1 --branch {TAG} {REPOSITORY} {source}"
        )
        .run()?;
    }
    let head = checkout_head(sh, &source).unwrap_or_default();
    ensure!(
        head.trim() == COMMIT,
        "llama.cpp {TAG} resolved to {head}, not the pinned {COMMIT}; delete {} and check the tag",
        source.display()
    );

    let build_number = TAG.trim_start_matches('b');
    let short_commit = &COMMIT[..8];
    cmd!(
        sh,
        "cmake -S {source} -B {build} {configure...} -DLLAMA_BUILD_NUMBER={build_number} -DLLAMA_BUILD_COMMIT={short_commit}"
    )
    .run()?;
    let jobs = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .to_string();
    cmd!(
        sh,
        "cmake --build {build} --config Release --target llama-server -j {jobs}"
    )
    .run()?;

    let built = build.join("bin").join("llama-server");
    check_system_libraries_only(sh, &built)?;
    fs::create_dir_all(binaries_dir).context("create binaries/")?;
    fs::copy(&built, &dst)
        .with_context(|| format!("copy {} -> {}", built.display(), dst.display()))?;
    fs::write(&stamp, expected_stamp).context("write build stamp")?;
    println!("prepare-binaries: binaries/librelog-llama-server-{triple}");
    Ok(())
}

/// The app bundles one file, so the server must not load llama.cpp or ggml
/// libraries from next to itself.
/// The commit checked out at `source`, if `source` is itself a git checkout
/// and not a directory inside another repository.
fn checkout_head(sh: &Shell, source: &Path) -> Option<String> {
    let top = cmd!(sh, "git -C {source} rev-parse --show-toplevel")
        .quiet()
        .ignore_stderr()
        .read()
        .ok()?;
    if fs::canonicalize(top.trim()).ok()? != fs::canonicalize(source).ok()? {
        return None;
    }
    cmd!(sh, "git -C {source} rev-parse HEAD")
        .quiet()
        .ignore_stderr()
        .read()
        .ok()
        .map(|head| head.trim().to_string())
}

fn check_system_libraries_only(sh: &Shell, binary: &Path) -> Result<()> {
    if cfg!(target_os = "linux") {
        return check_linux_libraries(sh, binary);
    }
    let linked = cmd!(sh, "otool -L {binary}").read()?;
    let foreign: Vec<&str> = linked
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.starts_with("/usr/lib/") && !line.starts_with("/System/Library/"))
        .collect();
    if !foreign.is_empty() {
        bail!(
            "{} links libraries outside the system: {}",
            binary.display(),
            foreign.join(", ")
        );
    }
    Ok(())
}

/// glibc's own libraries and the Vulkan loader, which the packages depend on
/// (`libvulkan1`, `vulkan-loader`) and an AppImage takes from the system.
const LINUX_SYSTEM_LIBRARIES: &[&str] = &[
    "libc.so.6",
    "libm.so.6",
    "libpthread.so.0",
    "libdl.so.2",
    "librt.so.1",
    "ld-linux-x86-64.so.2",
    "libvulkan.so.1",
];

fn check_linux_libraries(sh: &Shell, binary: &Path) -> Result<()> {
    let dynamic = cmd!(sh, "readelf -d {binary}").read()?;
    let foreign: Vec<&str> = dynamic
        .lines()
        .filter(|line| line.contains("(NEEDED)"))
        .filter_map(|line| {
            line.split_once('[')?
                .1
                .split_once(']')
                .map(|(name, _)| name)
        })
        .filter(|name| !LINUX_SYSTEM_LIBRARIES.contains(name))
        .collect();
    if !foreign.is_empty() {
        bail!(
            "{} links libraries outside glibc and the Vulkan loader: {}",
            binary.display(),
            foreign.join(", ")
        );
    }
    ensure!(
        !dynamic.contains("(RPATH)") && !dynamic.contains("(RUNPATH)"),
        "{} has an RPATH and could load libraries from next to itself",
        binary.display()
    );
    Ok(())
}
