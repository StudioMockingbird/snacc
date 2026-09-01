#[unsafe(no_mangle)]
pub extern "C" fn snacc_user_itoa_len(value: i64) -> i64 {
    let mut buffer = itoa::Buffer::new();
    buffer.format(value).len() as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_user_arg_count() -> i64 {
    std::env::args().skip(1).count() as i64
}

#[cfg(test)]
mod tests {
    #[test]
    fn bridge_uses_itoa() {
        assert_eq!(super::snacc_user_itoa_len(12345), 5);
    }
}
