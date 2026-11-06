//! Give the `nar` binary an rpath to the OpenTUI shared library and copy the
//! library next to the executable so `target/<profile>/nar` runs standalone.
use std::env;
use std::path::PathBuf;

fn main() {
    let Ok(lib_dir) = env::var("DEP_OPENTUI_LIB_DIR") else { return };
    let dylib = env::var("DEP_OPENTUI_DYLIB").ok();
    let macos = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    let origin = if macos { "@executable_path" } else { "$ORIGIN" };
    println!("cargo:rustc-link-arg=-Wl,-rpath,{origin}");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{origin}/../lib");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{lib_dir}");
    if let Some(dylib) = dylib {
        // OUT_DIR = target/<profile>/build/nar-xxxx/out → target/<profile>
        let out = PathBuf::from(env::var("OUT_DIR").unwrap());
        if let Some(profile_dir) = out.ancestors().nth(3) {
            let src = PathBuf::from(&dylib);
            if let Some(name) = src.file_name() {
                let _ = std::fs::copy(&src, profile_dir.join(name));
            }
        }
    }
}
