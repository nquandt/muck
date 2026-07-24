//! Explicitly asks the OS-level allocator to release freed heap pages back to the kernel.
//!
//! Rust's default allocator on Linux is glibc's `malloc`, which for small/medium allocations
//! (the common case for individually pushed file contents — most source files are well under
//! the ~128KB `mmap` threshold glibc uses for "big enough to `mmap` directly and `munmap` on
//! free") manages memory in an arena via `brk`/`sbrk`. Freeing memory in that arena adds it to
//! a free list for reuse by future allocations in the same process — it does **not** shrink
//! the process's resident memory (`anon`, specifically) or hand pages back to the OS on its
//! own. For a workload that allocates a large, short-lived burst (see `store.rs`'s `pending`
//! buffer, cleared at the end of every `build()` call) and then never needs that much heap
//! again, this shows up as resident memory that looks "used" from the OS's perspective but is
//! actually just retained-for-reuse free space glibc is holding onto.
//!
//! `malloc_trim(0)` is glibc's explicit "please give back what you can" call. It's
//! Linux/glibc-specific (not available, and not needed the same way, on other platforms —
//! Windows' heap and jemalloc/mimalloc-style allocators have different, generally more
//! proactive, release behavior), so this is a no-op everywhere else.

/// Call after freeing a large, short-lived heap allocation (e.g. `store.rs`'s push-phase
/// buffer, after a `build()` call clears it) to encourage the OS-visible resident memory
/// figure to actually reflect what's still live, not what was once allocated.
pub fn trim_heap() {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `malloc_trim` is a plain glibc libc function taking a `size_t` pad
        // argument and returning an `int`; declaring and calling it via FFI has no
        // preconditions beyond glibc actually being the allocator in use (true for a
        // standard Rust binary on Linux, which is what `cfg(target_os = "linux")` already
        // gates on) — passing `0` (no padding kept) is the documented "trim everything you
        // can" usage.
        unsafe {
            extern "C" {
                fn malloc_trim(pad: usize) -> i32;
            }
            malloc_trim(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Not a memory-effect test (that needs a real allocator under real pressure to observe —
    /// see the live-container measurement in `HANDOFF_STORAGE_OPTIMIZATION.md` for the actual
    /// before/after) — just confirms the FFI call is safe to make at all, repeatedly, on
    /// whatever platform the test suite runs on (a no-op off Linux).
    #[test]
    fn trim_heap_does_not_panic() {
        trim_heap();
        trim_heap();
    }
}
