//! Locates or builds `libopentui` (the OpenTUI Zig core) and links it.
//!
//! Resolution order:
//! 1. `OPENTUI_LIB_DIR` – a directory that already contains `libopentui.{so,dylib}`.
//! 2. `OPENTUI_SRC_DIR` or the `vendor/opentui` git submodule – built with
//!    `zig build` (requires Zig 0.17).
//!
//! The library directory is exported to dependents as `DEP_OPENTUI_LIB_DIR` so
//! the binary crate can set an rpath and copy the shared library next to the
//! executable.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn dylib_name() -> &'static str {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        "libopentui.dylib"
    } else {
        "libopentui.so"
    }
}

fn zig_target() -> Option<String> {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").ok()?;
    let os = env::var("CARGO_CFG_TARGET_OS").ok()?;
    let envv = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let arch = match arch.as_str() {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => return None,
    };
    Some(match os.as_str() {
        "linux" if envv == "musl" => format!("{arch}-linux-musl"),
        "linux" => format!("{arch}-linux-gnu"),
        "macos" => format!("{arch}-macos"),
        _ => return None,
    })
}

fn find_lib(root: &Path) -> Option<PathBuf> {
    let name = dylib_name();
    let target = zig_target().unwrap_or_default();
    let wanted_dir = target.replace("-gnu", "");
    let mut candidates = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let p = e.path().join(name);
            if p.exists() {
                candidates.push(e.path());
            }
        }
    }
    candidates.sort();
    candidates
        .iter()
        .find(|p| p.file_name().map(|n| n.to_string_lossy() == wanted_dir).unwrap_or(false))
        .or_else(|| candidates.iter().find(|p| p.to_string_lossy().contains(&target)))
        .or_else(|| candidates.first())
        .cloned()
}

fn build_from_source(src: &Path) -> PathBuf {
    let native = src.join("packages").join("native");
    if !native.join("build.zig").exists() {
        panic!(
            "OpenTUI sources not found at {}.\n\
             Run `git submodule update --init --depth 1 vendor/opentui`, or set OPENTUI_SRC_DIR,\n\
             or set OPENTUI_LIB_DIR to a directory containing {}.",
            src.display(),
            dylib_name()
        );
    }
    let lib_root = native.join("lib");
    if let Some(dir) = find_lib(&lib_root) {
        if env::var_os("OPENTUI_REBUILD").is_none() {
            return dir;
        }
    }
    let zig = env::var("ZIG").unwrap_or_else(|_| "zig".into());
    let ok = Command::new("sh")
        .arg("scripts/prepare-zig-deps.sh")
        .current_dir(&native)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        panic!("failed to unpack OpenTUI zig dependencies (scripts/prepare-zig-deps.sh)");
    }
    let mut cmd = Command::new(&zig);
    cmd.arg("build").arg("-Doptimize=ReleaseFast").current_dir(&native);
    if let Some(t) = zig_target() {
        cmd.arg(format!("-Dlibrary-target={t}"));
    }
    let status = cmd.status().unwrap_or_else(|e| {
        panic!("failed to run `{zig} build` for OpenTUI ({e}). Install Zig 0.17 or set OPENTUI_LIB_DIR.")
    });
    if !status.success() {
        panic!("`zig build` for OpenTUI failed");
    }
    find_lib(&lib_root).unwrap_or_else(|| panic!("zig build finished but no {} under {}", dylib_name(), lib_root.display()))
}

fn main() {
    println!("cargo:rerun-if-env-changed=OPENTUI_LIB_DIR");
    println!("cargo:rerun-if-env-changed=OPENTUI_SRC_DIR");
    println!("cargo:rerun-if-env-changed=OPENTUI_REBUILD");
    println!("cargo:rerun-if-changed=build.rs");

    let lib_dir = if let Ok(dir) = env::var("OPENTUI_LIB_DIR") {
        let dir = PathBuf::from(dir);
        if !dir.join(dylib_name()).exists() {
            panic!("OPENTUI_LIB_DIR={} does not contain {}", dir.display(), dylib_name());
        }
        dir
    } else {
        let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        let src = env::var("OPENTUI_SRC_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| manifest.join("..").join("..").join("vendor").join("opentui"));
        build_from_source(&src)
    };
    let lib_dir = lib_dir.canonicalize().unwrap_or(lib_dir);
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=opentui");
    // This crate's own tests and examples.
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
    println!("cargo:lib_dir={}", lib_dir.display());
    println!("cargo:dylib={}", lib_dir.join(dylib_name()).display());
}
