use ariadne::{Color, Label, Report, ReportKind, sources};
use snacc_compiler::{DiagnosticPhase, Diagnostics};
use std::{
    env, fs,
    path::{Path, PathBuf},
};
const UNKNOWN_ERROR: &str = "Unknown Error - you have found a bug in the compiler";

struct Cli {
    input: String,
    output: Option<String>,
}

/// Expected failures are reported as diagnostics; this hook labels invariant
/// failures that escape the compiler API as compiler bugs instead of source errors.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let msg = match info.payload().downcast_ref::<&str>() {
            Some(s) => s.to_string(),
            None => match info.payload().downcast_ref::<String>() {
                Some(s) => s.clone(),
                None => "(no message)".to_string(),
            },
        };
        eprintln!();
        eprintln!("{UNKNOWN_ERROR}");
        eprintln!("  {msg}");
        if let Some(loc) = info.location() {
            eprintln!("  at {}:{}:{}", loc.file(), loc.line(), loc.column());
        }
        eprintln!("  set RUST_BACKTRACE=1 to see a full stack trace.");
    }));
}

fn parse_cli(args: Vec<String>) -> Result<Cli, String> {
    let mut input = None;
    let mut output = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "-o" {
            if output.is_some() {
                return Err("option '-o' was provided more than once".into());
            }
            index += 1;
            if index == args.len() || args[index].starts_with('-') {
                return Err("option '-o' requires an output path".into());
            }
            output = Some(args[index].clone());
        } else if arg.starts_with('-') {
            return Err(format!("unknown option '{arg}'"));
        } else if input.is_some() {
            return Err("only one input file may be provided".into());
        } else {
            input = Some(arg.clone());
        }
        index += 1;
    }

    let Some(input) = input else {
        return Err("no input file was provided".into());
    };
    Ok(Cli { input, output })
}

fn report_cli_error(message: &str) {
    eprintln!("snacc: {message}");
    eprintln!("usage: snacc <input.nrs> [-o <output>]");
}

fn report_unknown_error(detail: &str) {
    eprintln!("{UNKNOWN_ERROR}");
    eprintln!("  {detail}");
}

fn report_diagnostics(filename: &str, source: &str, diagnostics: &Diagnostics) {
    for diagnostic in &diagnostics.items {
        if diagnostic.phase == DiagnosticPhase::Internal {
            report_unknown_error(&diagnostic.message);
            continue;
        }
        let Some(span) = &diagnostic.span else {
            eprintln!("snacc: {}", diagnostic.message);
            continue;
        };
        Report::build(ReportKind::Error, (filename.to_string(), span.clone()))
            .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
            .with_message(&diagnostic.message)
            .with_label(
                Label::new((filename.to_string(), span.clone()))
                    .with_message(&diagnostic.message)
                    .with_color(Color::Red),
            )
            .finish()
            .eprint(sources([(filename.to_string(), source)]))
            .unwrap();
    }
}

fn main() {
    install_panic_hook();

    let cli = match parse_cli(env::args().skip(1).collect()) {
        Ok(cli) => cli,
        Err(message) => {
            report_cli_error(&message);
            std::process::exit(1);
        }
    };
    let input = cli.input;

    let src = match fs::read_to_string(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to read '{input}': {e}");
            std::process::exit(1);
        }
    };

    let stem = PathBuf::from(&input)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "out".into());
    let exe_path = cli
        .output
        .unwrap_or_else(|| format!("{stem}{}", env::consts::EXE_SUFFIX));
    match snacc_driver::build_to(&src, Path::new(&exe_path)) {
        Ok(()) => println!("wrote {exe_path}"),
        Err(snacc_driver::DriverError::Compile(diagnostics)) => {
            report_diagnostics(&input, &src, &diagnostics);
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parses_one_input_and_output() {
        let cli = parse_cli(args(&["program.nrs", "-o", "program.exe"])).unwrap();
        assert_eq!(cli.input, "program.nrs");
        assert_eq!(cli.output.as_deref(), Some("program.exe"));
    }

    #[test]
    fn rejects_invalid_arguments() {
        assert_eq!(
            parse_cli(args(&["--unknown", "program.nrs"]))
                .err()
                .unwrap(),
            "unknown option '--unknown'"
        );
        assert!(parse_cli(args(&["first.nrs", "second.nrs"])).is_err());
        assert!(parse_cli(args(&["program.nrs", "-o"])).is_err());
        assert!(parse_cli(args(&["program.nrs", "-o", "a", "-o", "b"])).is_err());
    }

    #[test]
    fn unknown_error_is_the_root_compiler_bug_message() {
        assert_eq!(
            UNKNOWN_ERROR,
            "Unknown Error - you have found a bug in the compiler"
        );
    }
}
