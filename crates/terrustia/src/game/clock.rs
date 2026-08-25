//! How much processor a stretch of code actually used.
//!
//! Timing a tick with the wall clock answers the wrong question. A tick that takes 26 ms of wall
//! clock has usually not done 26 ms of work: it has done a fraction of a millisecond of work and
//! spent the rest descheduled, because the machine is also running a game, a compiler, or a
//! backup. Reporting that as "ticks are using a lot of their budget" sends somebody hunting for a
//! slow routine that does not exist.
//!
//! The thread clock counts only the time this thread was on a core, so it measures the server's
//! own cost. Both numbers are worth having — work that overruns is a bug in here, wall clock that
//! overruns without the work to match is the machine being busy elsewhere — so the game loop
//! records both and says which one it is.

use std::time::Duration;

/// A reading of the current thread's CPU clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cpu(Duration);

impl Cpu {
    /// Read the calling thread's consumed CPU time.
    ///
    /// The one place in the whole workspace that needs `unsafe`, and the allow is per-function so
    /// it stays an explicit exception rather than a blanket permission: there is no safe way to
    /// ask an operating system how much processor a thread has used.
    #[cfg(unix)]
    #[allow(unsafe_code)]
    pub fn now() -> Self {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `clock_gettime` writes a `timespec` through the pointer and reads nothing else.
        let ok = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) } == 0;
        if !ok {
            // A clock that cannot be read must not make every tick look free or infinitely slow;
            // zero means the CPU check simply never fires and the wall clock still reports.
            return Self(Duration::ZERO);
        }
        Self(Duration::new(
            ts.tv_sec.max(0) as u64,
            ts.tv_nsec.clamp(0, 999_999_999) as u32,
        ))
    }

    /// Read the calling thread's consumed CPU time.
    ///
    /// Windows has no `clock_gettime` and no `CLOCK_THREAD_CPUTIME_ID` — neither symbol exists in
    /// libc's Windows module — so the `unix` version above does not merely misbehave here, it
    /// fails to compile. This was the *only* thing stopping the whole project building on Windows.
    ///
    /// `GetThreadTimes` is the equivalent, reporting kernel and user time as 100-nanosecond ticks
    /// in a pair of `FILETIME`s. Both are wanted: a tick that spends its time in a write syscall
    /// costs the server just as much as one that spends it in a loop.
    #[cfg(windows)]
    #[allow(unsafe_code)]
    pub fn now() -> Self {
        use std::mem::MaybeUninit;

        // Minimal declarations rather than a dependency on the whole Windows API surface, matching
        // how the unix side uses `libc` directly. `FILETIME` is two 32-bit halves of a 64-bit
        // count of 100ns intervals.
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct FileTime {
            low: u32,
            high: u32,
        }
        unsafe extern "system" {
            fn GetCurrentThread() -> isize;
            fn GetThreadTimes(
                thread: isize,
                creation: *mut FileTime,
                exit: *mut FileTime,
                kernel: *mut FileTime,
                user: *mut FileTime,
            ) -> i32;
        }

        let mut creation = MaybeUninit::<FileTime>::uninit();
        let mut exit = MaybeUninit::<FileTime>::uninit();
        let mut kernel = MaybeUninit::<FileTime>::uninit();
        let mut user = MaybeUninit::<FileTime>::uninit();

        // SAFETY: `GetThreadTimes` writes four `FILETIME`s through the pointers and reads nothing
        // else; the handle from `GetCurrentThread` is a pseudo-handle that is always valid and
        // needs no closing. The values are only read when it reports success.
        let ok = unsafe {
            GetThreadTimes(
                GetCurrentThread(),
                creation.as_mut_ptr(),
                exit.as_mut_ptr(),
                kernel.as_mut_ptr(),
                user.as_mut_ptr(),
            ) != 0
        };
        if !ok {
            return Self(Duration::ZERO);
        }
        // SAFETY: written by the call above, which reported success.
        let (kernel, user) = unsafe { (kernel.assume_init(), user.assume_init()) };
        let ticks = |t: FileTime| (u64::from(t.high) << 32) | u64::from(t.low);
        Self(Duration::from_nanos(
            (ticks(kernel) + ticks(user)).saturating_mul(100),
        ))
    }

    /// CPU time consumed since an earlier reading.
    pub fn since(self, earlier: Self) -> Duration {
        self.0.saturating_sub(earlier.0)
    }
}

#[cfg(test)]
mod tests {
    use super::Cpu;
    use std::time::Duration;

    /// Sleeping burns wall clock and no CPU, which is the whole reason this module exists.
    #[test]
    fn sleeping_costs_no_processor_time() {
        let before = Cpu::now();
        std::thread::sleep(Duration::from_millis(30));
        let used = Cpu::now().since(before);
        assert!(
            used < Duration::from_millis(5),
            "sleeping should not look like work, got {used:?}"
        );
    }

    /// Work does show up, so the clock is not simply stuck at zero.
    #[test]
    fn work_costs_processor_time() {
        let before = Cpu::now();
        let mut total = 0u64;
        for i in 0..4_000_000u64 {
            total = total.wrapping_add(i * i);
        }
        std::hint::black_box(total);
        let used = Cpu::now().since(before);
        assert!(
            used > Duration::ZERO,
            "four million multiplies cost nothing?"
        );
    }
}
