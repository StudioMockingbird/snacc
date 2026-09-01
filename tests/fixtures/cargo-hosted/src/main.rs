mod interop;

unsafe extern "C" {
    fn snacc_main() -> i32;
}

fn main() {
    snacc_runtime::force_link();
    // SAFETY: cargo-snacc links this host with the object defining this ABI.
    let status = unsafe { snacc_main() };
    std::process::exit(status);
}

#[test]
fn snacc_entry_succeeds() {
    snacc_runtime::force_link();
    // SAFETY: cargo-snacc links this harness with the object defining this ABI.
    assert_eq!(unsafe { snacc_main() }, 0);
}
