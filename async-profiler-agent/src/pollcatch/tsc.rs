/// Current timestamp, async-signal-safe
#[inline]
pub fn now() -> u64 {
    _now()
}

#[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
#[inline]
fn _now() -> u64 {
    unsafe { ::core::arch::x86_64::_rdtsc() }
}

#[cfg(target_arch = "aarch64")]
#[inline]
fn _now() -> u64 {
    let count: u64;

    unsafe {
        ::core::arch::asm!("mrs {}, cntvct_el0", out(reg) count);
    }

    count
}

#[cfg(not(any(
    all(target_arch = "x86_64", target_feature = "sse2"),
    target_arch = "aarch64",
)))]
#[inline]
fn _now() -> u64 {
    0
}
