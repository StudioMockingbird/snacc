//! Stable Rust-side ABI used by generated Snacc objects.

/// Contract version for generated objects, runtime imports, and Rust bridges.
pub const ABI_VERSION: u32 = 4;

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

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_u8(value: u8) {
    println!("{value}");
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_u16(value: u16) {
    println!("{value}");
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_u32(value: u32) {
    println!("{value}");
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_u64(value: u64) {
    println!("{value}");
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_f32(value: f32) {
    println!("{value}");
}

#[doc(hidden)]
#[inline(never)]
pub fn force_link() {
    let symbols = [
        snacc_print_f64 as *const () as usize,
        snacc_print_i64 as *const () as usize,
        snacc_print_bool as *const () as usize,
        snacc_print_nil as *const () as usize,
        snacc_print_u8 as *const () as usize,
        snacc_print_u16 as *const () as usize,
        snacc_print_u32 as *const () as usize,
        snacc_print_u64 as *const () as usize,
        snacc_print_f32 as *const () as usize,
    ];
    std::hint::black_box(symbols);
}
