//! Random bytes from the operating system, for the salt and the build seed (the TS transformer's
//! `crypto.randomBytes` and `uuid.v4`). Windows: `ProcessPrng`, the generator Rust's own standard library uses;
//! elsewhere `/dev/urandom`. No crate: none in the lock provides it.

/// Fills `buf` with cryptographically secure random bytes. Panics if the system has no generator.
pub fn fill(buf: &mut [u8]) {
    imp::fill(buf)
}

/// `crypto.randomBytes(n).toString("hex")`.
pub fn hex(n: usize) -> String {
    let mut buf = vec![0u8; n];
    fill(&mut buf);
    crate::sha::hex(&buf)
}

#[cfg(windows)]
mod imp {
    #[link(name = "bcryptprimitives", kind = "raw-dylib")]
    unsafe extern "system" {
        fn ProcessPrng(data: *mut u8, len: usize) -> i32;
    }

    pub fn fill(buf: &mut [u8]) {
        // SAFETY: ProcessPrng writes exactly `len` bytes to `data`, which `buf` provides; it always returns TRUE.
        let ok = unsafe { ProcessPrng(buf.as_mut_ptr(), buf.len()) };
        assert!(ok != 0, "ProcessPrng failed");
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io::Read;

    pub fn fill(buf: &mut [u8]) {
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(buf))
            .expect("/dev/urandom");
    }
}
