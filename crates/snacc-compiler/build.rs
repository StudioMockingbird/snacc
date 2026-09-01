#[cfg(windows)]
fn main() {
    use std::{path::PathBuf, process::Command};

    println!("cargo:rerun-if-env-changed=LLVM_SYS_221_PREFIX");
    println!("cargo:rerun-if-changed=../../.cargo/config.toml");
    let llvm_root = match std::env::var_os("LLVM_SYS_221_PREFIX") {
        Some(root) => PathBuf::from(root),
        None => PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("../../vendor/clang+llvm-22.1.8-x86_64-pc-windows-msvc"),
    };
    let llvm_root = std::fs::canonicalize(&llvm_root).unwrap_or_else(|error| {
        panic!(
            "LLVM 22 prefix '{}' cannot be resolved: {error}; set LLVM_SYS_221_PREFIX to a valid LLVM 22.1.8 installation",
            llvm_root.display()
        )
    });
    let required = [
        llvm_root.join("bin/llvm-config.exe"),
        llvm_root.join("bin/LLVM-C.dll"),
        llvm_root.join("include/llvm-c/Target.h"),
    ];
    for path in required {
        if !path.is_file() {
            panic!(
                "LLVM 22 prefix '{}' is missing '{}'; install the LLVM 22.1.8 developer distribution",
                llvm_root.display(),
                path.display()
            );
        }
    }
    let version = Command::new(llvm_root.join("bin/llvm-config.exe"))
        .arg("--version")
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "failed to run llvm-config from '{}': {error}",
                llvm_root.display()
            )
        });
    if !version.status.success() {
        panic!(
            "llvm-config from '{}' failed with {}",
            llvm_root.display(),
            version.status
        );
    }
    let version = String::from_utf8_lossy(&version.stdout);
    if !version.trim().starts_with("22.1.8") {
        panic!(
            "LLVM 22.1.8 is required, but '{}' reports '{}'; set LLVM_SYS_221_PREFIX to a matching installation",
            llvm_root.display(),
            version.trim()
        );
    }

    let lib_dir = llvm_root.join("lib");
    let llvm_c = lib_dir.join("LLVM-C.lib");
    if !llvm_c.is_file() {
        panic!(
            "LLVM 22 library not found at '{}'; set LLVM_SYS_221_PREFIX to the LLVM installation",
            llvm_c.display()
        );
    }

    // `no-llvm-linking` leaves native library selection to this package, so
    // the validated monolithic C API import library is linked explicitly.
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=LLVM-C");
}

#[cfg(not(windows))]
fn main() {}
