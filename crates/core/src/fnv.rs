//! FNV-1a, stable across Rust versions and platforms, unlike `DefaultHasher`,
//! so hashes can be stored and compared later.

pub(crate) struct Fnv(u64);

impl Fnv {
    pub(crate) fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    pub(crate) fn finish(&self) -> String {
        format!("{:016x}", self.0)
    }
}
