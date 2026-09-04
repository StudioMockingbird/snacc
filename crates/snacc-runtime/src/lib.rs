//! Stable Rust-side ABI used by generated Snacc objects.

use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};

/// Contract version for generated objects, runtime imports, and Rust bridges.
pub const ABI_VERSION: u32 = 7;

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

/// Specification 016 section 8.2: `box(expression)` lowers to a call to this
/// allocator, which either returns valid, non-null, `align`-aligned storage
/// for `size` bytes or terminates the process -- never a null pointer, an
/// error code, or any other recoverable outcome.
///
/// A zero-sized pointee (Specification 016 section 8.2's closing paragraph)
/// is special-cased to a fixed non-null, well-aligned sentinel rather than
/// calling the global allocator, whose own safety contract forbids a
/// zero-size request (`std::alloc::alloc`'s documentation: "Undefined
/// behavior [...] if layout has zero size"). This is the same
/// never-read-or-written dangling-but-valid convention Rust's own
/// `Layout`/`NonNull::dangling()` use for a zero-sized type, so no real
/// allocation, and therefore no matching `snacc_dealloc` call, ever happens
/// for it -- `snacc_dealloc` mirrors this same `size == 0` special case.
#[unsafe(no_mangle)]
pub extern "C" fn snacc_alloc(size: usize, align: usize) -> *mut u8 {
    if size == 0 {
        return align.max(1) as *mut u8;
    }
    let layout = Layout::from_size_align(size, align).unwrap_or_else(|_| {
        panic!("snacc: invalid allocation request (size {size}, align {align})")
    });
    // Safety: `size` is non-zero, checked above, and `layout` was just built
    // by `Layout::from_size_align`, so it satisfies `alloc`'s only
    // requirement (non-zero size).
    let ptr = unsafe { alloc(layout) };
    if ptr.is_null() {
        handle_alloc_error(layout);
    }
    ptr
}

/// Releases one allocation `snacc_alloc` returned for the exact same `size`
/// and `align` (Specification 016 section 8.1: a box releases its allocation
/// on normal destruction). A `size == 0` request never reaches the global
/// allocator here either, mirroring `snacc_alloc`'s own zero-size sentinel,
/// which must never be passed to `dealloc`.
#[unsafe(no_mangle)]
pub extern "C" fn snacc_dealloc(ptr: *mut u8, size: usize, align: usize) {
    if size == 0 {
        return;
    }
    let layout = Layout::from_size_align(size, align).unwrap_or_else(|_| {
        panic!("snacc: invalid allocation request (size {size}, align {align})")
    });
    // Safety: the checked cleanup plan that emits this call always pairs it
    // with the `snacc_alloc` call that produced `ptr`, using that same
    // pointee's size and alignment, so `ptr` was allocated by the global
    // allocator with this exact `layout` and has not been freed already.
    unsafe { dealloc(ptr, layout) };
}

#[doc(hidden)]
#[inline(never)]
pub fn force_link() {
    let symbols = [
        snacc_print_f64 as *const () as usize,
        snacc_print_i64 as *const () as usize,
        snacc_print_bool as *const () as usize,
        snacc_print_u8 as *const () as usize,
        snacc_print_u16 as *const () as usize,
        snacc_print_u32 as *const () as usize,
        snacc_print_u64 as *const () as usize,
        snacc_print_f32 as *const () as usize,
        snacc_alloc as *const () as usize,
        snacc_dealloc as *const () as usize,
    ];
    std::hint::black_box(symbols);
}
