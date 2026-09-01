//! Stable Rust-side ABI used by generated Snacc objects.

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_f64(value: f64) {
    println!("{value}");
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_i64(value: i64) {
    println!("{value}");
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_bool(value: u8) {
    println!("{}", value != 0);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_nil() {
    println!("nil");
}

#[doc(hidden)]
#[inline(never)]
pub fn force_link() {
    let symbols = [
        snacc_print_f64 as *const () as usize,
        snacc_print_i64 as *const () as usize,
        snacc_print_bool as *const () as usize,
        snacc_print_nil as *const () as usize,
    ];
    std::hint::black_box(symbols);
}
