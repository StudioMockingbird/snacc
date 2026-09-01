fn main() {
    if let Err(error) = snacc_workbench::run() {
        eprintln!("snacc-workbench: {error}");
        std::process::exit(1);
    }
}
