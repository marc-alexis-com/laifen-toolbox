//! Ctrl+C / SIGTERM handling: long waits notice it, so that the brush is disconnected
//! instead of being left connected to the computer.

use std::sync::atomic::{AtomicBool, Ordering};

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    // Only an atomic store: anything else is not async-signal-safe.
    STOP.store(true, Ordering::SeqCst);
}

pub fn install() {
    // SAFETY: the handler only performs an atomic store.
    unsafe {
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
        libc::signal(libc::SIGHUP, handler);
    }
}

pub fn stop_requested() -> bool {
    STOP.load(Ordering::SeqCst)
}
