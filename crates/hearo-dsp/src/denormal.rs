//! Flush-to-zero guard. Denormal (subnormal) floats appear when IIR filter tails decay towards
//! zero; on many CPUs they are 10–100× slower to compute, which can blow the audio deadline.
//! Apple Silicon handles them in hardware at full speed in most cases, but flushing is still the
//! audio-industry default and makes results identical across machines.
//!
//! Caveat: Rust/LLVM formally assume the default floating-point environment, so changing FPCR
//! is outside the language model (the reason `_mm_setcsr` is deprecated). We keep the window
//! narrow and the `msr` instructions are *not* `nomem`, so they act as compiler barriers and FP
//! work in opaque module calls can't be moved across them. hEARo targets Apple Silicon only
//! (HANDOVER §1.2), so there is no x86 MXCSR path.

/// Sets flush-to-zero for the current thread while alive; restores the previous mode on drop.
/// Real-time safe (two register accesses).
#[derive(Debug)]
pub struct DenormalGuard {
    #[cfg(target_arch = "aarch64")]
    previous: u64,
}

#[cfg(target_arch = "aarch64")]
const FPCR_FZ: u64 = 1 << 24;

impl DenormalGuard {
    /// Enable flush-to-zero until the guard is dropped.
    #[inline]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        #[cfg(target_arch = "aarch64")]
        {
            let previous: u64;
            // SAFETY: reading and writing FPCR only changes floating-point rounding/flush
            // behaviour of this thread; FZ is a documented, user-writable bit.
            unsafe {
                core::arch::asm!("mrs {}, fpcr", out(reg) previous, options(nostack, preserves_flags));
                core::arch::asm!("msr fpcr, {}", in(reg) previous | FPCR_FZ, options(nostack, preserves_flags));
            }
            Self { previous }
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            Self {}
        }
    }
}

impl Drop for DenormalGuard {
    #[inline]
    fn drop(&mut self) {
        #[cfg(target_arch = "aarch64")]
        // SAFETY: restores the value read in `new`.
        unsafe {
            core::arch::asm!("msr fpcr, {}", in(reg) self.previous, options(nostack, preserves_flags));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;

    #[inline(never)]
    fn mul(a: f32, b: f32) -> f32 {
        black_box(black_box(a) * black_box(b))
    }

    #[test]
    #[cfg(target_arch = "aarch64")]
    fn flushes_inside_and_restores_after() {
        let tiny = 1e-39f32;
        assert!(mul(tiny, 1.0) != 0.0, "subnormals exist by default");
        {
            let _g = DenormalGuard::new();
            assert_eq!(mul(tiny, 1.0), 0.0, "flushed under the guard");
        }
        assert!(mul(tiny, 1.0) != 0.0, "mode restored after drop");
    }
}
