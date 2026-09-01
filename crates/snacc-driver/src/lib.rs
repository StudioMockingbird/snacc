use snacc_compiler::Diagnostics;
use std::{
    fmt, fs,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::{Builder, TempDir};

const RUNTIME_SOURCE: &str = include_str!("../../snacc-runtime/src/lib.rs");
const HOST_SUFFIX: &str = r#"

unsafe extern "C" {
    fn snacc_main() -> i32;
}

fn main() {
    force_link();
    // SAFETY: the linked Snacc object defines snacc_main with this ABI.
    let status = unsafe { snacc_main() };
    std::process::exit(status);
}
"#;

#[derive(Debug)]
pub enum DriverError {
    Compile(Diagnostics),
    Filesystem(String),
    Tool(String),
}

impl fmt::Display for DriverError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(diagnostics) => {
                write!(formatter, "compiler diagnostics: {diagnostics:?}")
            }
            Self::Filesystem(message) | Self::Tool(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for DriverError {}

pub struct BuiltExecutable {
    directory: TempDir,
    path: PathBuf,
}

impl BuiltExecutable {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn directory(&self) -> &Path {
        self.directory.path()
    }
}

pub fn build(source: &str) -> Result<BuiltExecutable, DriverError> {
    let directory = Builder::new()
        .prefix("snacc-build-")
        .tempdir()
        .map_err(|error| {
            DriverError::Filesystem(format!("failed to create native build directory: {error}"))
        })?;
    let object_path = directory.path().join("program.o");
    let host_path = directory.path().join("host.rs");
    let executable_path = directory
        .path()
        .join(format!("program{}", std::env::consts::EXE_SUFFIX));

    let emitted = snacc_compiler::emit_object(source).map_err(DriverError::Compile)?;
    fs::write(&object_path, emitted).map_err(|error| {
        DriverError::Filesystem(format!("failed to write native object: {error}"))
    })?;
    fs::write(&host_path, format!("{RUNTIME_SOURCE}{HOST_SUFFIX}")).map_err(|error| {
        DriverError::Filesystem(format!("failed to write generated host: {error}"))
    })?;

    let status = Command::new("rustc")
        .arg("--edition=2024")
        .arg(&host_path)
        .arg("-C")
        .arg(format!("link-arg={}", object_path.display()))
        .arg("-o")
        .arg(&executable_path)
        .status()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                DriverError::Tool(
                    "rustc was not found on PATH; install a Rust toolchain to link Snacc programs"
                        .into(),
                )
            } else {
                DriverError::Tool(format!("failed to run rustc: {error}"))
            }
        })?;
    if !status.success() {
        return Err(DriverError::Tool(format!(
            "linking with 'rustc' failed (exit {status})"
        )));
    }

    Ok(BuiltExecutable {
        directory,
        path: executable_path,
    })
}

pub fn build_to(source: &str, output: &Path) -> Result<(), DriverError> {
    let executable = build(source)?;
    fs::copy(executable.path(), output).map_err(|error| {
        DriverError::Filesystem(format!(
            "failed to place executable at '{}': {error}",
            output.display()
        ))
    })?;
    Ok(())
}
