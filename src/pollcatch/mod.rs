use std::{
    cell::Cell,
    sync::{
        atomic::{self, AtomicBool},
        LazyLock,
    },
};

mod tsc;

use crate::asprof::{self, AsProf};

static POLLCATCH_JFR_KEY: LazyLock<Option<asprof::UserJfrKey>> = LazyLock::new(|| {
    // Pollcatch V1 event contains:
    //  8 byte little-endian "before" tsc timestamp
    //  8 byte little-endian "after" tsc timestamp
    AsProf::create_user_jfr_key(c"tokio.PollcatchV1")
        .map_err(|e| {
            tracing::warn!(message="error creating jfr key", error=?e);
        })
        .ok()
});

static EMITTED_JFR_ERROR: AtomicBool = AtomicBool::new(false);

#[cold]
#[inline(never)]
fn write_timestamp(before: u64) {
    if let Some(key) = *POLLCATCH_JFR_KEY {
        let end = tsc::now();
        let mut buf = [0u8; 16];

        buf[0..8].copy_from_slice(&before.to_le_bytes()[..]);
        buf[8..16].copy_from_slice(&end.to_le_bytes()[..]);
        if let Err(e) = AsProf::emit_user_jfr(key, &buf) {
            if !EMITTED_JFR_ERROR.swap(true, atomic::Ordering::Relaxed) {
                tracing::warn!(message="error emitting jfr", error=?e);
            }
        }
    }
}

thread_local! {
    static BEFORE_POLL_TIMESTAMP: Cell<u64> = const { Cell::new(0) };
    static BEFORE_POLL_SAMPLE_COUNTER: Cell<u64> = const { Cell::new(0) };
}

/// Call this in the Tokio before task hook
pub fn before_poll_hook() {
    let before = tsc::now();
    BEFORE_POLL_TIMESTAMP.set(before);
    BEFORE_POLL_SAMPLE_COUNTER.set(AsProf::get_sample_counter().unwrap_or(0));
}

/// Call this in the Tokio after task hook
pub fn after_poll_hook() {
    let sample_counter = AsProf::get_sample_counter().unwrap_or(0);
    if sample_counter != BEFORE_POLL_SAMPLE_COUNTER.get() {
        write_timestamp(BEFORE_POLL_TIMESTAMP.get());
    }
}
