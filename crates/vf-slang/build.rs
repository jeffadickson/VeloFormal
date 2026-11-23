use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let slang_dir = workspace_root.join("third_party").join("slang");
    let cpp_dir = manifest_dir.join("cpp");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let build_dir = out_dir.join("cmake-build");

    assert!(
        slang_dir.join("CMakeLists.txt").exists(),
        "third_party/slang submodule not found at {}. Run `git submodule update --init --recursive`.",
        slang_dir.display()
    );

    println!("cargo:rerun-if-changed={}", cpp_dir.join("CMakeLists.txt").display());
    println!("cargo:rerun-if-changed={}", cpp_dir.join("slang_bridge.h").display());
    println!("cargo:rerun-if-changed={}", cpp_dir.join("slang_bridge.cpp").display());

    let use_ninja = Command::new("ninja")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    let mut cfg = Command::new("cmake");
    cfg.arg("-S")
        .arg(&cpp_dir)
        .arg("-B")
        .arg(&build_dir)
        .arg(format!("-DSLANG_SRC_DIR={}", slang_dir.display()))
        .arg("-DCMAKE_BUILD_TYPE=Release");
    if use_ninja {
        cfg.arg("-G").arg("Ninja");
    }
    let status = cfg.status().expect("failed to execute cmake configure");
    assert!(status.success(), "cmake configure failed for vf-slang");

    let mut build = Command::new("cmake");
    build
        .arg("--build")
        .arg(&build_dir)
        .arg("--target")
        .arg("vf_slang_bridge")
        .arg("--parallel");
    let status = build.status().expect("failed to execute cmake build");
    assert!(status.success(), "cmake build failed for vf-slang");

    // Collect every directory containing built static archives so linking works across
    // Ninja / Make / MSVC generators and FetchContent layouts.
    let mut lib_dirs = Vec::new();
    find_static_lib_dirs(&build_dir, &mut lib_dirs);
    lib_dirs.sort();
    lib_dirs.dedup();
    for dir in &lib_dirs {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }

    println!("cargo:rustc-link-lib=static=vf_slang_bridge");
    println!("cargo:rustc-link-lib=static=svlang");

    // Link fmt if FetchContent built a local libfmt.a, otherwise link system fmt if needed.
    if has_archive(&lib_dirs, "libfmt.a") || has_archive(&lib_dirs, "fmt.lib") {
        println!("cargo:rustc-link-lib=static=fmt");
    } else if has_archive(&lib_dirs, "libfmtd.a") {
        println!("cargo:rustc-link-lib=static=fmtd");
    }

    #[cfg(target_os = "linux")]
    {
        println!("cargo:rustc-link-lib=dylib=stdc++");
        println!("cargo:rustc-link-lib=dylib=pthread");
        println!("cargo:rustc-link-lib=dylib=m");
    }
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=dylib=c++");
    }
}

fn find_static_lib_dirs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_static_lib_dirs(&path, out);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if ext == "a" || ext == "lib" {
                if let Some(parent) = path.parent() {
                    out.push(parent.to_path_buf());
                }
            }
        }
    }
}

fn has_archive(dirs: &[PathBuf], file_name: &str) -> bool {
    dirs.iter().any(|d| d.join(file_name).exists())
}
