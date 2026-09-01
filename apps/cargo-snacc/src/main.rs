use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use snacc_compiler::{
    CompileOptions, Diagnostics, Optimization, Program, Ty, check, emit_object_with_options,
};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug)]
struct CliError(String);

#[derive(Default)]
struct Options {
    package: Option<String>,
    release: bool,
    profile: Option<String>,
    features: Vec<String>,
    all_features: bool,
    no_default_features: bool,
    locked: bool,
    frozen: bool,
    offline: bool,
    verbose: bool,
    all: bool,
}

struct ParsedOptions {
    options: Options,
    positionals: Vec<String>,
    trailing: Vec<String>,
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    target_directory: PathBuf,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    manifest_path: PathBuf,
    targets: Vec<Target>,
    metadata: Option<Value>,
}

#[derive(Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
    src_path: PathBuf,
}

struct Selected {
    package: Package,
    package_id: String,
    entry: PathBuf,
    host_bin: String,
    host_src_path: PathBuf,
    target_directory: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct SnaccMetadata {
    schema_version: u32,
    entry: String,
    host_bin: String,
}

#[derive(Deserialize, Serialize)]
struct CacheManifest {
    schema: u32,
    identity: String,
    compiler_version: String,
    backend_build_id: String,
    abi_version: u32,
    llvm_version: String,
    target_triple: String,
    optimization: String,
    profile: String,
    object_sha256: String,
}

fn main() {
    if let Err(error) = run(env::args().skip(1).collect()) {
        eprintln!("cargo-snacc: {}", error.0);
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<(), CliError> {
    let args = args.strip_prefix(&[String::from("snacc")]).unwrap_or(&args);
    let Some(command) = args.first().map(String::as_str) else {
        return Err(CliError(
            "expected one of init, check, build, run, test, clean, doctor".into(),
        ));
    };
    match command {
        "init" => init(&args[1..]),
        "check" => check_command(&args[1..]),
        "build" => build_command(&args[1..], false),
        "run" => build_command(&args[1..], true),
        "test" => test_command(&args[1..]),
        "clean" => clean_command(&args[1..]),
        "doctor" => doctor(),
        _ => Err(CliError(format!("unknown command '{command}'"))),
    }
}

fn cargo() -> String {
    env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

fn init(args: &[String]) -> Result<(), CliError> {
    let mut path = PathBuf::from(".");
    let mut name = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--name" => {
                index += 1;
                name = args.get(index).cloned();
                if name.is_none() {
                    return Err(CliError("--name requires a value".into()));
                }
            }
            value if value.starts_with('-') => {
                return Err(CliError(format!("unknown init option '{value}'")));
            }
            value if path != Path::new(".") => {
                return Err(CliError(format!("unexpected init argument '{value}'")));
            }
            value => path = PathBuf::from(value),
        }
        index += 1;
    }
    let mut command = Command::new(cargo());
    command.arg("init").arg(&path).arg("--vcs").arg("none");
    if let Some(name) = &name {
        command.arg("--name").arg(name);
    }
    let status = command
        .status()
        .map_err(|error| CliError(format!("failed to run cargo init: {error}")))?;
    if !status.success() {
        return Err(CliError(format!("cargo init failed with {status}")));
    }
    let root = fs::canonicalize(&path).map_err(io_error)?;
    let manifest = root.join("Cargo.toml");
    let mut cargo_toml = fs::read_to_string(&manifest).map_err(io_error)?;
    if cargo_toml.contains("[package.metadata.snacc]") {
        return Err(CliError(format!(
            "refusing to reinitialize existing Snacc package '{}'",
            root.display()
        )));
    }
    let src = root.join("src");
    let main_nrs = src.join("main.nrs");
    let main_rs = src.join("main.rs");
    let interop_rs = src.join("interop.rs");
    ensure_new_path(&main_nrs)?;
    ensure_new_path(&interop_rs)?;
    ensure_cargo_main_template(&main_rs)?;

    if cargo_toml.contains("[dependencies]") {
        cargo_toml = cargo_toml.replacen(
            "[dependencies]",
            "[dependencies]\nsnacc-runtime = \"0.1\"",
            1,
        );
    } else {
        cargo_toml.push_str("\n[dependencies]\nsnacc-runtime = \"0.1\"\n");
    }
    cargo_toml.push_str(
        "\n[package.metadata.snacc]\nschema-version = 1\nentry = \"src/main.nrs\"\nhost-bin = \"",
    );
    let package_name = name
        .or_else(|| {
            root.file_name()
                .map(|value| value.to_string_lossy().into_owned())
        })
        .ok_or_else(|| CliError("cannot determine package name".into()))?;
    cargo_toml.push_str(&package_name);
    cargo_toml.push_str("\"\n");
    fs::write(&manifest, cargo_toml).map_err(io_error)?;
    fs::write(&main_nrs, "print(0)\n").map_err(io_error)?;
    fs::write(
        &main_rs,
        "mod interop;\n\nunsafe extern \"C\" {\n    fn snacc_main() -> i32;\n}\n\nfn main() {\n    snacc_runtime::force_link();\n    // SAFETY: cargo-snacc links this host with the object defining this ABI.\n    let status = unsafe { snacc_main() };\n    std::process::exit(status);\n}\n\n#[test]\nfn snacc_entry_succeeds() {\n    snacc_runtime::force_link();\n    // SAFETY: cargo-snacc links this harness with the object defining this ABI.\n    assert_eq!(unsafe { snacc_main() }, 0);\n}\n",
    )
    .map_err(io_error)?;
    fs::write(
        &interop_rs,
        "// This module exports explicitly selected Rust APIs through Snacc's C-compatible ABI.\n",
    )
    .map_err(io_error)?;
    println!("initialized Snacc package at {}", root.display());
    Ok(())
}

fn check_command(args: &[String]) -> Result<(), CliError> {
    let parsed = parse_options(args)?;
    reject_extra(&parsed, "check")?;
    let options = parsed.options;
    let selected = select_package(options.package.as_deref(), Some(&options))?;
    let source = fs::read_to_string(&selected.entry).map_err(io_error)?;
    check(&source)
        .map_err(|diagnostics| diagnostic_error(&selected.entry, &source, &diagnostics))?;
    let mut command = Command::new(cargo());
    command
        .arg("check")
        .arg("--manifest-path")
        .arg(selected.package.manifest_path)
        .arg("--package")
        .arg(&selected.package.name);
    append_cargo_options(&mut command, &options);
    run_forwarded(command, "cargo check")
}

fn build_command(args: &[String], run_program: bool) -> Result<(), CliError> {
    let parsed = parse_options(args)?;
    if !parsed.positionals.is_empty() {
        return Err(CliError(format!(
            "unexpected argument '{}'",
            parsed.positionals[0]
        )));
    }
    if !run_program && !parsed.trailing.is_empty() {
        return Err(CliError(
            "build does not accept arguments after '--'".into(),
        ));
    }
    let options = parsed.options;
    let program_args = parsed.trailing;
    let selected = select_package(options.package.as_deref(), Some(&options))?;
    let source = fs::read_to_string(&selected.entry).map_err(io_error)?;
    let object = emit_cached(&selected, &source, &options)?;

    let mut command = Command::new(cargo());
    command
        .arg("rustc")
        .arg("--message-format=json-render-diagnostics")
        .arg("--manifest-path")
        .arg(&selected.package.manifest_path)
        .arg("--package")
        .arg(&selected.package.name)
        .arg("--bin")
        .arg(&selected.host_bin);
    append_cargo_options(&mut command, &options);
    command
        .arg("--")
        .arg("-C")
        .arg(format!("link-arg={}", object.display()));
    let output = command
        .output()
        .map_err(|error| CliError(format!("failed to run cargo rustc: {error}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    print!("{stderr}");
    render_cargo_messages(&stdout);
    if !output.status.success() {
        return Err(CliError(format!(
            "cargo rustc failed with {}",
            output.status
        )));
    }
    let executable = cargo_executable(&stdout, &selected, &selected.host_bin, "bin")
        .ok_or_else(|| CliError("cargo produced no executable artifact".into()))?;
    println!("Snacc executable: {}", executable.display());
    if run_program {
        let status = Command::new(&executable)
            .args(program_args)
            .status()
            .map_err(io_error)?;
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
    }
    Ok(())
}

fn clean_command(args: &[String]) -> Result<(), CliError> {
    let parsed = parse_options(args)?;
    reject_extra(&parsed, "clean")?;
    let options = parsed.options;
    if options.all {
        let mut command = Command::new(cargo());
        command.arg("clean");
        if let Some(package) = options.package {
            command.arg("--package").arg(package);
        }
        return run_forwarded(command, "cargo clean");
    }
    let selected = select_package(options.package.as_deref(), Some(&options))?;
    let state = selected.target_directory.join("snacc");
    if state.exists() {
        fs::remove_dir_all(&state).map_err(io_error)?;
    }
    println!("removed Snacc artifacts from {}", state.display());
    Ok(())
}

fn test_command(args: &[String]) -> Result<(), CliError> {
    let parsed = parse_options(args)?;
    if parsed.positionals.len() > 1 {
        return Err(CliError("test accepts at most one filter".into()));
    }
    let filter = parsed.positionals.first().cloned();
    let test_args = parsed.trailing;
    let options = parsed.options;
    let selected = select_package(options.package.as_deref(), Some(&options))?;
    let source = fs::read_to_string(&selected.entry).map_err(io_error)?;
    let object = emit_cached(&selected, &source, &options)?;

    let mut command = Command::new(cargo());
    command
        .arg("rustc")
        .arg("--message-format=json-render-diagnostics")
        .arg("--manifest-path")
        .arg(&selected.package.manifest_path)
        .arg("--package")
        .arg(&selected.package.name)
        .arg("--bin")
        .arg(&selected.host_bin);
    append_cargo_options(&mut command, &options);
    command
        .arg("--")
        .arg("--test")
        .arg("-C")
        .arg(format!("link-arg={}", object.display()));
    let output = command
        .output()
        .map_err(|error| CliError(format!("failed to build Snacc integration test: {error}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    print!("{}", String::from_utf8_lossy(&output.stderr));
    render_cargo_messages(&stdout);
    if !output.status.success() {
        return Err(CliError(format!(
            "Snacc integration test build failed with {}",
            output.status
        )));
    }
    let executable = cargo_executable(&stdout, &selected, &selected.host_bin, "bin")
        .ok_or_else(|| CliError("Cargo produced no Snacc host test executable".into()))?;
    let mut test = Command::new(executable);
    if let Some(filter) = filter {
        test.arg(filter);
    }
    test.args(test_args);
    let status = test.status().map_err(io_error)?;
    if status.success() {
        Ok(())
    } else {
        Err(CliError(format!(
            "Snacc integration tests failed with {status}"
        )))
    }
}

fn doctor() -> Result<(), CliError> {
    let mut failures = Vec::new();
    if let Err(error) = command_version(&cargo()) {
        failures.push(error.0);
    }
    let rustc = command_output("rustc", &["-vV"]);
    let rustc_host = match rustc {
        Ok(output) => {
            println!(
                "{}",
                output.lines().next().unwrap_or("rustc version unavailable")
            );
            output
                .lines()
                .find_map(|line| line.strip_prefix("host: "))
                .map(str::to_owned)
        }
        Err(error) => {
            failures.push(error.0);
            None
        }
    };
    let llvm_target = snacc_compiler::target_triple();
    if let Some(rustc_host) = rustc_host {
        if rustc_host != llvm_target {
            failures.push(format!(
                "rustc host '{rustc_host}' does not match LLVM target '{llvm_target}'"
            ));
        }
    }
    let llvm_version = snacc_compiler::llvm_version();
    if llvm_version != (22, 1, 8) {
        failures.push(format!(
            "loaded LLVM version is {}.{}.{}, expected 22.1.8",
            llvm_version.0, llvm_version.1, llvm_version.2
        ));
    }
    let dll = if let Some(prefix) = env::var_os("LLVM_SYS_221_PREFIX") {
        PathBuf::from(prefix).join("bin/LLVM-C.dll")
    } else {
        env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.join("LLVM-C.dll")))
            .unwrap_or_else(|| PathBuf::from("LLVM-C.dll"))
    };
    if !dll.is_file() {
        failures.push(format!("LLVM runtime DLL is missing: {}", dll.display()));
    } else {
        println!("LLVM runtime: {}", dll.display());
        #[cfg(windows)]
        match loaded_llvm_path() {
            Ok(loaded) => match (fs::canonicalize(&dll), fs::canonicalize(&loaded)) {
                (Ok(expected), Ok(loaded)) => {
                    if !expected
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&loaded.to_string_lossy())
                    {
                        failures.push(format!(
                            "loaded LLVM runtime '{}' does not match selected '{}'",
                            loaded.display(),
                            expected.display()
                        ));
                    }
                }
                (Err(error), _) => failures.push(format!(
                    "cannot resolve selected LLVM runtime '{}': {error}",
                    dll.display()
                )),
                (_, Err(error)) => failures.push(format!(
                    "cannot resolve loaded LLVM runtime '{}': {error}",
                    loaded.display()
                )),
            },
            Err(error) => failures.push(error.0),
        }
    }
    if let Err(error) = probe_linker() {
        failures.push(error.0);
    }
    if let Some(manifest) = find_manifest(&env::current_dir().map_err(io_error)?) {
        let manifest_source = fs::read_to_string(&manifest).map_err(io_error)?;
        if manifest_source.contains("[package.metadata.snacc]") {
            match select_package_optional(None, None) {
                Ok(Some(_)) => {}
                Ok(None) => failures.push("package.metadata.snacc is missing".into()),
                Err(error) => failures.push(error.0),
            }
        } else {
            println!(
                "doctor: current Cargo package is not a Snacc application; package checks skipped"
            );
        }
    }
    if failures.is_empty() {
        println!("doctor: all checks passed");
        Ok(())
    } else {
        Err(CliError(format!(
            "doctor found {} problem(s):\n- {}",
            failures.len(),
            failures.join("\n- ")
        )))
    }
}

fn find_manifest(start: &Path) -> Option<PathBuf> {
    let mut directory = Some(start);
    while let Some(path) = directory {
        let manifest = path.join("Cargo.toml");
        if manifest.is_file() {
            return Some(manifest);
        }
        directory = path.parent();
    }
    None
}

fn probe_linker() -> Result<(), CliError> {
    let directory = tempfile::Builder::new()
        .prefix("snacc-doctor-")
        .tempdir()
        .map_err(io_error)?;
    let output = directory.path().join(if cfg!(windows) {
        "link-probe.exe"
    } else {
        "link-probe"
    });
    let mut child = Command::new("rustc")
        .arg("-")
        .arg("--crate-name")
        .arg("snacc_link_probe")
        .arg("-o")
        .arg(output)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| CliError(format!("failed to start native linker probe: {error}")))?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| CliError("native linker probe has no stdin".into()))?
        .write_all(b"fn main() {}\n")
        .map_err(io_error)?;
    let output = child
        .wait_with_output()
        .map_err(|error| CliError(format!("failed to wait for native linker probe: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CliError(format!(
            "native linker probe failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

#[cfg(windows)]
fn loaded_llvm_path() -> Result<PathBuf, CliError> {
    use std::ffi::{OsStr, c_void};
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetModuleFileNameW(module: *mut c_void, path: *mut u16, size: u32) -> u32;
    }

    let name = OsStr::new("LLVM-C.dll")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut path = vec![0u16; 32768];
    // `doctor` queries LLVM before this probe, so the module handle is live;
    // these calls only borrow it and initialize at most `path.len()` units.
    let length = unsafe {
        let module = GetModuleHandleW(name.as_ptr());
        if module.is_null() {
            return Err(CliError("LLVM-C.dll is not loaded in this process".into()));
        }
        GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32)
    };
    if length == 0 || length as usize == path.len() {
        return Err(CliError(
            "Windows could not resolve the loaded LLVM-C.dll path".into(),
        ));
    }
    path.truncate(length as usize);
    Ok(PathBuf::from(String::from_utf16_lossy(&path)))
}

fn select_package(
    requested: Option<&str>,
    options: Option<&Options>,
) -> Result<Selected, CliError> {
    select_package_optional(requested, options)?
        .ok_or_else(|| CliError("package.metadata.snacc is missing".into()))
}

fn select_package_optional(
    requested: Option<&str>,
    options: Option<&Options>,
) -> Result<Option<Selected>, CliError> {
    let root = env::current_dir().map_err(io_error)?;
    let mut command = Command::new(cargo());
    command
        .arg("metadata")
        .arg("--format-version")
        .arg("1")
        .arg("--no-deps")
        .current_dir(&root);
    if let Some(options) = options {
        append_metadata_options(&mut command, options);
    }
    let output = command
        .output()
        .map_err(|error| CliError(format!("failed to run cargo metadata: {error}")))?;
    if !output.status.success() {
        return Err(CliError(
            String::from_utf8_lossy(&output.stderr).trim().into(),
        ));
    }
    let metadata: Metadata = serde_json::from_slice(&output.stdout)
        .map_err(|error| CliError(format!("invalid cargo metadata JSON: {error}")))?;
    let package = if let Some(name) = requested {
        metadata
            .packages
            .into_iter()
            .find(|package| package.name == name)
            .ok_or_else(|| CliError(format!("package '{name}' was not found")))?
    } else {
        let canonical = fs::canonicalize(&root).map_err(io_error)?;
        let mut packages = metadata.packages;
        let exact = packages.iter().position(|package| {
            package.manifest_path.parent().is_some_and(|parent| {
                fs::canonicalize(parent)
                    .ok()
                    .is_some_and(|parent| parent == canonical)
            })
        });
        let mut matches = if let Some(index) = exact {
            vec![packages.swap_remove(index)]
        } else {
            packages
                .into_iter()
                .filter(|package| {
                    package
                        .manifest_path
                        .parent()
                        .and_then(|parent| fs::canonicalize(parent).ok())
                        .is_some_and(|parent| canonical.starts_with(parent))
                })
                .collect::<Vec<_>>()
        };
        if matches.len() != 1 {
            return Err(CliError(
                "package selection is ambiguous; use --package".into(),
            ));
        }
        matches.remove(0)
    };
    let Some(metadata_value) = package
        .metadata
        .as_ref()
        .and_then(|value| value.get("snacc"))
    else {
        return Ok(None);
    };
    let snacc_metadata: SnaccMetadata = serde_json::from_value(metadata_value.clone())
        .map_err(|error| CliError(format!("invalid package.metadata.snacc: {error}")))?;
    if snacc_metadata.schema_version != 1 {
        return Err(CliError(format!(
            "unsupported package.metadata.snacc schema-version {}",
            snacc_metadata.schema_version
        )));
    }
    let entry = snacc_metadata.entry;
    let host_bin = snacc_metadata.host_bin;
    let package_root = package
        .manifest_path
        .parent()
        .ok_or_else(|| CliError("package manifest has no parent".into()))?;
    let package_root = fs::canonicalize(package_root).map_err(io_error)?;
    let entry = fs::canonicalize(package_root.join(&entry)).map_err(io_error)?;
    if !entry.starts_with(package_root) || !entry.is_file() {
        return Err(CliError("Snacc entry must be a package-owned file".into()));
    }
    let host_target = package
        .targets
        .iter()
        .filter(|target| target.name == host_bin && target.kind.iter().any(|kind| kind == "bin"))
        .collect::<Vec<_>>();
    if host_target.len() != 1 {
        return Err(CliError(format!(
            "host binary '{host_bin}' does not resolve to exactly one binary target"
        )));
    }
    let host_src_path = host_target[0].src_path.clone();
    let package_id = package.id.clone();
    Ok(Some(Selected {
        package,
        package_id,
        entry,
        host_bin,
        host_src_path,
        target_directory: metadata.target_directory,
    }))
}

fn emit_cached(selected: &Selected, source: &str, options: &Options) -> Result<PathBuf, CliError> {
    let release = options.release || options.profile.as_deref() == Some("release");
    let compile_options = CompileOptions {
        optimization: if release {
            Optimization::Aggressive
        } else {
            Optimization::None
        },
    };
    let target_triple = snacc_compiler::target_triple();
    let llvm_version = snacc_compiler::llvm_version();
    let llvm_version = format!("{}.{}.{}", llvm_version.0, llvm_version.1, llvm_version.2);
    let backend_build_id = backend_build_id();
    let profile =
        options
            .profile
            .as_deref()
            .unwrap_or(if options.release { "release" } else { "dev" });
    let mut hash = Sha256::new();
    hash.update(env!("CARGO_PKG_VERSION"));
    hash.update(b"snacc-llvm-object-v1");
    hash.update(&backend_build_id);
    hash.update(snacc_compiler::ABI_VERSION.to_le_bytes());
    hash.update(&llvm_version);
    hash.update(&target_triple);
    hash.update(compile_options.optimization.as_str());
    hash.update(profile);
    hash.update(package_relative_entry(selected).to_string_lossy().as_bytes());
    hash.update(source.as_bytes());
    let identity = format!("{:x}", hash.finalize());
    let directory = selected
        .target_directory
        .join("snacc")
        .join(&target_triple)
        .join(profile)
        .join(&selected.package.name)
        .join(&identity);
    let object = directory.join(if cfg!(windows) { "app.obj" } else { "app.o" });
    let manifest = directory.join("manifest.json");
    if cache_matches(
        &object,
        &manifest,
        &identity,
        &backend_build_id,
        &llvm_version,
        &target_triple,
        compile_options.optimization,
        profile,
    )? {
        if options.verbose {
            println!("Snacc object reused: {}", object.display());
        }
        return Ok(object);
    }
    let emitted = emit_object_with_options(source, compile_options)
        .map_err(|diagnostics| diagnostic_error(&selected.entry, source, &diagnostics))?;
    if emitted.target_triple != target_triple {
        return Err(CliError(format!(
            "compiler emitted target '{}' after selecting '{}'",
            emitted.target_triple, target_triple
        )));
    }
    fs::create_dir_all(&directory).map_err(io_error)?;
    let temp = directory.join("app.tmp");
    fs::write(&temp, &emitted.bytes).map_err(io_error)?;
    fs::rename(&temp, &object).map_err(io_error)?;
    let cache = CacheManifest {
        schema: 1,
        identity: identity.clone(),
        compiler_version: env!("CARGO_PKG_VERSION").into(),
        backend_build_id,
        abi_version: emitted.abi_version,
        llvm_version,
        target_triple,
        optimization: emitted.optimization.as_str().into(),
        profile: profile.into(),
        object_sha256: sha256(&emitted.bytes),
    };
    let manifest_temp = directory.join("manifest.tmp");
    let encoded = serde_json::to_vec_pretty(&cache)
        .map_err(|error| CliError(format!("encode cache manifest: {error}")))?;
    fs::write(&manifest_temp, encoded).map_err(io_error)?;
    fs::rename(&manifest_temp, &manifest).map_err(io_error)?;
    if options.verbose {
        println!("Snacc object rebuilt: {}", selected.entry.display());
    }
    Ok(object)
}

fn package_relative_entry(selected: &Selected) -> &Path {
    selected
        .entry
        .strip_prefix(
            selected
                .package
                .manifest_path
                .parent()
                .unwrap_or(Path::new(".")),
        )
        .unwrap_or(&selected.entry)
}

fn rust_abi_type(ty: Ty) -> &'static str {
    match ty {
        Ty::Int64 => "i64",
        Ty::Dec64 => "f64",
        Ty::Bool | Ty::Nil => "u8",
    }
}

fn render_bridge_assertions(checked: &Program, entry: &Path, source: &str) -> String {
    let mut names: Vec<&String> = checked.externs.keys().collect();
    names.sort();
    let mut rendered = String::from(
        "// Generated by cargo-snacc from checked `extern rust` declarations (RFC 007).\n\
         // Do not edit; this file is regenerated on every build.\n",
    );
    for name in names {
        let extern_decl = &checked.externs[name];
        let params = extern_decl
            .params
            .iter()
            .map(|(_, ty)| rust_abi_type(*ty))
            .collect::<Vec<_>>()
            .join(", ");
        let (line, column) = line_column(source, extern_decl.span.start);
        rendered.push_str(&format!(
            "const _: unsafe extern \"C\" fn({params}) -> {ret} = crate::interop::{symbol}; // snacc: {name} ({entry}:{line}:{column})\n",
            ret = rust_abi_type(extern_decl.ret),
            symbol = extern_decl.symbol,
            entry = entry.display(),
        ));
    }
    rendered
}

fn write_bridge_assertions(
    selected: &Selected,
    checked: &Program,
    source: &str,
) -> Result<PathBuf, CliError> {
    let content = render_bridge_assertions(checked, package_relative_entry(selected), source);
    let mut hash = Sha256::new();
    hash.update(selected.package_id.as_bytes());
    hash.update(content.as_bytes());
    let digest = format!("{:x}", hash.finalize());
    let directory = selected.target_directory.join("snacc").join("bridges");
    fs::create_dir_all(&directory).map_err(io_error)?;
    let path = directory.join(format!("{}-{}.rs", selected.package.name, digest));
    if !path.is_file() {
        let mut temp = tempfile::Builder::new()
            .prefix(&format!("{}-{}-", selected.package.name, digest))
            .suffix(".rs.tmp")
            .tempfile_in(&directory)
            .map_err(io_error)?;
        temp.write_all(content.as_bytes()).map_err(io_error)?;
        if let Err(error) = temp.persist(&path) {
            if !path.is_file() {
                return Err(CliError(format!(
                    "failed to publish bridge assertions: {error}"
                )));
            }
        }
    }
    Ok(path)
}

fn backend_build_id() -> String {
    let mut hash = Sha256::new();
    hash.update(include_bytes!(
        "../../../crates/snacc-compiler/src/syntax/ast.rs"
    ));
    hash.update(include_bytes!(
        "../../../crates/snacc-compiler/src/syntax/lexer.rs"
    ));
    hash.update(include_bytes!(
        "../../../crates/snacc-compiler/src/syntax/parser.rs"
    ));
    hash.update(include_bytes!(
        "../../../crates/snacc-compiler/src/semantics/checker.rs"
    ));
    hash.update(include_bytes!(
        "../../../crates/snacc-compiler/src/backend/llvm.rs"
    ));
    hash.update(include_bytes!("../../../crates/snacc-runtime/src/lib.rs"));
    format!("{:x}", hash.finalize())
}

#[allow(clippy::too_many_arguments)]
fn cache_matches(
    object: &Path,
    manifest: &Path,
    identity: &str,
    backend_build_id: &str,
    llvm_version: &str,
    target_triple: &str,
    optimization: Optimization,
    profile: &str,
) -> Result<bool, CliError> {
    if !object.is_file() || !manifest.is_file() {
        return Ok(false);
    }
    let bytes = fs::read(object).map_err(io_error)?;
    if bytes.is_empty() {
        return Ok(false);
    }
    let encoded = fs::read(manifest).map_err(io_error)?;
    let Ok(cache) = serde_json::from_slice::<CacheManifest>(&encoded) else {
        return Ok(false);
    };
    Ok(cache.schema == 1
        && cache.identity == identity
        && cache.compiler_version == env!("CARGO_PKG_VERSION")
        && cache.backend_build_id == backend_build_id
        && cache.abi_version == snacc_compiler::ABI_VERSION
        && cache.llvm_version == llvm_version
        && cache.target_triple == target_triple
        && cache.optimization == optimization.as_str()
        && cache.profile == profile
        && cache.object_sha256 == sha256(&bytes))
}

fn sha256(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(bytes);
    format!("{:x}", hash.finalize())
}

fn diagnostic_error(path: &Path, source: &str, diagnostics: &Diagnostics) -> CliError {
    let mut rendered = String::new();
    for diagnostic in &diagnostics.items {
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str(&path.display().to_string());
        if let Some(span) = &diagnostic.span {
            let (line, column) = line_column(source, span.start);
            rendered.push_str(&format!(":{line}:{column}"));
        }
        rendered.push_str(&format!(
            ": {:?} error: {}",
            diagnostic.phase, diagnostic.message
        ));
    }
    if rendered.is_empty() {
        rendered.push_str("Snacc compilation failed without a diagnostic");
    }
    CliError(rendered)
}

fn line_column(source: &str, offset: usize) -> (usize, usize) {
    let mut bounded = offset.min(source.len());
    while bounded > 0 && !source.is_char_boundary(bounded) {
        bounded -= 1;
    }
    let prefix = &source[..bounded];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, tail)| tail)
        .chars()
        .count()
        + 1;
    (line, column)
}

fn parse_options(args: &[String]) -> Result<ParsedOptions, CliError> {
    let mut options = Options::default();
    let mut positionals = Vec::new();
    let mut trailing = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            trailing.extend_from_slice(&args[index + 1..]);
            break;
        }
        match arg.as_str() {
            "--release" => options.release = true,
            "--all-features" => options.all_features = true,
            "--no-default-features" => options.no_default_features = true,
            "--locked" => options.locked = true,
            "--frozen" => options.frozen = true,
            "--offline" => options.offline = true,
            "--all" => options.all = true,
            "--verbose" | "-v" => options.verbose = true,
            "--package" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(CliError("--package requires a value".into()));
                };
                if value.starts_with('-') {
                    return Err(CliError("--package requires a package name".into()));
                }
                options.package = Some(value.clone());
            }
            "--profile" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(CliError("--profile requires a value".into()));
                };
                if value.starts_with('-') {
                    return Err(CliError("--profile requires a profile name".into()));
                }
                options.profile = Some(value.clone());
            }
            "--features" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(CliError("--features requires a value".into()));
                };
                if value.starts_with('-') {
                    return Err(CliError("--features requires a feature list".into()));
                }
                options.features = value.split(',').map(str::to_owned).collect();
            }
            value if value.starts_with('-') => {
                return Err(CliError(format!("unknown option '{value}'")));
            }
            value => positionals.push(value.to_owned()),
        }
        index += 1;
    }
    if options.release && options.profile.is_some() {
        return Err(CliError(
            "--release and --profile are mutually exclusive".into(),
        ));
    }
    if let Some(profile) = &options.profile {
        if profile != "dev" && profile != "release" {
            return Err(CliError(format!("unsupported Cargo profile '{profile}'")));
        }
    }
    Ok(ParsedOptions {
        options,
        positionals,
        trailing,
    })
}

fn reject_extra(parsed: &ParsedOptions, command: &str) -> Result<(), CliError> {
    if let Some(value) = parsed.positionals.first() {
        return Err(CliError(format!("unexpected {command} argument '{value}'")));
    }
    if !parsed.trailing.is_empty() {
        return Err(CliError(format!(
            "{command} does not accept arguments after '--'"
        )));
    }
    Ok(())
}

fn append_cargo_options(command: &mut Command, options: &Options) {
    if options.release {
        command.arg("--release");
    }
    if let Some(profile) = &options.profile {
        command.arg("--profile").arg(profile);
    }
    if options.all_features {
        command.arg("--all-features");
    }
    if options.no_default_features {
        command.arg("--no-default-features");
    }
    if !options.features.is_empty() {
        command.arg("--features").arg(options.features.join(","));
    }
    if options.locked {
        command.arg("--locked");
    }
    if options.frozen {
        command.arg("--frozen");
    }
    if options.offline {
        command.arg("--offline");
    }
}

fn append_metadata_options(command: &mut Command, options: &Options) {
    if options.all_features {
        command.arg("--all-features");
    }
    if options.no_default_features {
        command.arg("--no-default-features");
    }
    if !options.features.is_empty() {
        command.arg("--features").arg(options.features.join(","));
    }
    if options.locked {
        command.arg("--locked");
    }
    if options.frozen {
        command.arg("--frozen");
    }
    if options.offline {
        command.arg("--offline");
    }
}

fn cargo_executable(
    stdout: &str,
    selected: &Selected,
    target_name: &str,
    target_kind: &str,
) -> Option<PathBuf> {
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("reason").and_then(Value::as_str) != Some("compiler-artifact") {
            continue;
        }
        let package_matches = value
            .get("package_id")
            .and_then(Value::as_str)
            .is_some_and(|package_id| package_id == selected.package_id);
        let target = value.get("target");
        let name_matches = target
            .and_then(|target| target.get("name"))
            .and_then(Value::as_str)
            .is_some_and(|name| name == target_name);
        let kind_matches = target
            .and_then(|target| target.get("kind"))
            .and_then(Value::as_array)
            .is_some_and(|kinds| kinds.iter().any(|kind| kind.as_str() == Some(target_kind)));
        if package_matches && name_matches && kind_matches {
            return value
                .get("executable")
                .and_then(Value::as_str)
                .map(PathBuf::from);
        }
    }
    None
}

fn render_cargo_messages(stdout: &str) {
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            if !line.trim().is_empty() {
                eprintln!("{line}");
            }
            continue;
        };
        if value.get("reason").and_then(Value::as_str) != Some("compiler-message") {
            continue;
        }
        if let Some(rendered) = value
            .get("message")
            .and_then(|message| message.get("rendered"))
            .and_then(Value::as_str)
        {
            eprint!("{rendered}");
        }
    }
}

fn run_forwarded(mut command: Command, label: &str) -> Result<(), CliError> {
    let status = command
        .status()
        .map_err(|error| CliError(format!("failed to run {label}: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(CliError(format!("{label} failed with {status}")))
    }
}

fn command_version(name: &str) -> Result<(), CliError> {
    let status = Command::new(name)
        .arg("--version")
        .status()
        .map_err(|error| CliError(format!("{name} is unavailable: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(CliError(format!("{name} --version failed with {status}")))
    }
}

fn command_output(name: &str, args: &[&str]) -> Result<String, CliError> {
    let output = Command::new(name)
        .args(args)
        .output()
        .map_err(|error| CliError(format!("{name} is unavailable: {error}")))?;
    if !output.status.success() {
        return Err(CliError(format!(
            "{name} {} failed with {}: {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn ensure_new_path(path: &Path) -> Result<(), CliError> {
    if path.exists() {
        return Err(CliError(format!(
            "refusing to overwrite existing file '{}'",
            path.display()
        )));
    }
    Ok(())
}

fn ensure_cargo_main_template(path: &Path) -> Result<(), CliError> {
    if path.is_file() {
        let existing = fs::read_to_string(path).map_err(io_error)?;
        if existing.trim() != "fn main() {\n    println!(\"Hello, world!\");\n}" {
            return Err(CliError(format!(
                "refusing to overwrite non-template Rust host '{}'",
                path.display()
            )));
        }
    }
    Ok(())
}

fn io_error(error: std::io::Error) -> CliError {
    CliError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected() -> Selected {
        Selected {
            package: Package {
                id: "path+file:///workspace#app@0.1.0".into(),
                name: "app".into(),
                manifest_path: PathBuf::from("C:/workspace/Cargo.toml"),
                targets: vec![Target {
                    name: "app".into(),
                    kind: vec!["bin".into()],
                    src_path: PathBuf::from("C:/workspace/src/main.rs"),
                }],
                metadata: None,
            },
            package_id: "path+file:///workspace#app@0.1.0".into(),
            entry: PathBuf::from("C:/workspace/src/main.nrs"),
            host_bin: "app".into(),
            host_src_path: PathBuf::from("C:/workspace/src/main.rs"),
            target_directory: PathBuf::from("C:/workspace/target"),
        }
    }

    #[test]
    fn options_reject_conflicting_profiles() {
        let args = vec!["--release".into(), "--profile".into(), "dev".into()];
        assert!(parse_options(&args).is_err());
    }

    #[test]
    fn cargo_external_subcommand_prefix_is_accepted() {
        let error = run(vec!["snacc".into(), "unknown".into()]).unwrap_err();
        assert_eq!(error.0, "unknown command 'unknown'");
    }

    #[test]
    fn options_preserve_arguments_after_separator() {
        let args = vec!["--offline".into(), "--".into(), "one".into(), "two".into()];
        let parsed = parse_options(&args).unwrap();
        assert!(parsed.options.offline);
        assert_eq!(parsed.trailing, ["one", "two"]);
    }

    #[test]
    fn cargo_artifact_selection_requires_package_target_and_kind() {
        let messages = concat!(
            "{\"reason\":\"compiler-artifact\",\"package_id\":\"other\",\"target\":{\"name\":\"app\",\"kind\":[\"bin\"]},\"executable\":\"wrong.exe\"}\n",
            "{\"reason\":\"compiler-artifact\",\"package_id\":\"path+file:///workspace#app@0.1.0\",\"target\":{\"name\":\"app\",\"kind\":[\"bin\"]},\"executable\":\"right.exe\"}\n"
        );
        assert_eq!(
            cargo_executable(messages, &selected(), "app", "bin"),
            Some(PathBuf::from("right.exe"))
        );
    }

    #[test]
    fn selected_test_helper_carries_a_host_src_path() {
        assert_eq!(
            selected().host_src_path,
            PathBuf::from("C:/workspace/src/main.rs")
        );
    }

    #[test]
    fn metadata_schema_rejects_unknown_fields() {
        let result = serde_json::from_value::<SnaccMetadata>(serde_json::json!({
            "schema-version": 1,
            "entry": "src/main.nrs",
            "host-bin": "app",
            "unknown": true
        }));
        assert!(result.is_err());
    }

    #[test]
    fn diagnostic_rendering_preserves_all_messages_and_locations() {
        let diagnostics = Diagnostics {
            items: vec![
                snacc_compiler::Diagnostic {
                    phase: snacc_compiler::DiagnosticPhase::TypeCheck,
                    message: "first".into(),
                    span: Some(0..1),
                },
                snacc_compiler::Diagnostic {
                    phase: snacc_compiler::DiagnosticPhase::TypeCheck,
                    message: "second".into(),
                    span: Some(2..3),
                },
            ],
        };
        let rendered = diagnostic_error(Path::new("main.nrs"), "x\ny", &diagnostics).0;
        assert!(rendered.contains("main.nrs:1:1: TypeCheck error: first"));
        assert!(rendered.contains("main.nrs:2:1: TypeCheck error: second"));
    }

    #[test]
    fn init_preflight_rejects_existing_source_and_custom_host() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("main.nrs");
        fs::write(&source, "print(42)").unwrap();
        assert!(ensure_new_path(&source).is_err());

        let host = directory.path().join("main.rs");
        fs::write(&host, "fn main() { run_custom_host(); }").unwrap();
        assert!(ensure_cargo_main_template(&host).is_err());
        fs::write(&host, "fn main() {\n    println!(\"Hello, world!\");\n}\n").unwrap();
        assert!(ensure_cargo_main_template(&host).is_ok());
    }

    #[test]
    fn render_bridge_assertions_sorts_by_snacc_name_and_maps_types() {
        let source = concat!(
            "extern rust \"snacc_user_zeta\" fun zeta(value: Bool): Nil\n",
            "extern rust \"snacc_user_alpha\" fun alpha(a: Int64, b: Dec64): Bool\n",
            "print(0)\n"
        );
        let checked = check(source).expect("bridge declarations should type check");
        let rendered = render_bridge_assertions(&checked, Path::new("src/main.nrs"), source);
        let alpha_line = rendered
            .lines()
            .find(|line| line.contains("snacc_user_alpha"))
            .expect("alpha assertion line");
        let zeta_line = rendered
            .lines()
            .find(|line| line.contains("snacc_user_zeta"))
            .expect("zeta assertion line");
        assert!(rendered.find(alpha_line).unwrap() < rendered.find(zeta_line).unwrap());
        assert!(alpha_line.contains("fn(i64, f64) -> u8"));
        assert!(alpha_line.contains("crate::interop::snacc_user_alpha"));
        assert!(alpha_line.contains("// snacc: alpha (src/main.nrs:2:1)"));
        assert!(zeta_line.contains("fn(u8) -> u8"));
        assert!(zeta_line.contains("// snacc: zeta (src/main.nrs:1:1)"));
    }

    #[test]
    fn render_bridge_assertions_with_no_externs_is_header_only() {
        let source = "print(0)\n";
        let checked = check(source).expect("source should type check");
        let rendered = render_bridge_assertions(&checked, Path::new("src/main.nrs"), source);
        assert!(rendered.starts_with("// Generated by cargo-snacc"));
        assert!(!rendered.contains("const _:"));
    }

    #[test]
    fn render_bridge_assertions_is_deterministic() {
        let source = "extern rust \"snacc_user_a\" fun a(): Int64\nprint(0)\n";
        let checked = check(source).expect("source should type check");
        let first = render_bridge_assertions(&checked, Path::new("src/main.nrs"), source);
        let second = render_bridge_assertions(&checked, Path::new("src/main.nrs"), source);
        assert_eq!(first, second);
    }
}
