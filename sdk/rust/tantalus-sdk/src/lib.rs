pub const ABI_VERSION: i32 = 1;

pub fn unpack_buffer(packed: i64) -> (i32, i32) {
    let packed = packed as u64;
    (packed as u32 as i32, (packed >> 32) as u32 as i32)
}

pub fn pack_buffer(pointer: i32, length: i32) -> i64 {
    ((i64::from(length)) << 32) | i64::from(pointer)
}

/// # Safety
/// `packed` must be the live host-provided input buffer for the current callback.
pub unsafe fn input_bytes(packed: i64) -> &'static [u8] {
    let (pointer, length) = unpack_buffer(packed);
    unsafe { std::slice::from_raw_parts(pointer as *const u8, length as usize) }
}

pub fn output_bytes(bytes: Vec<u8>) -> i64 {
    let pointer = bytes.as_ptr() as i32;
    let length = bytes.len() as i32;
    std::mem::forget(bytes);
    pack_buffer(pointer, length)
}

/// # Safety
/// This must be called only in a WASM guest linked against Tantalus ABI version 1.
pub unsafe fn emit_json(bytes: &[u8]) {
    #[link(wasm_import_module = "tantalus")]
    unsafe extern "C" {
        #[link_name = "emit"]
        fn tantalus_emit(pointer: i32, length: i32);
    }
    unsafe { tantalus_emit(bytes.as_ptr() as i32, bytes.len() as i32) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_buffers_retain_pointer_and_length() {
        let packed = pack_buffer(4096, 327);
        assert_eq!(unpack_buffer(packed), (4096, 327));
    }
}
