extern crate async_profiler_agent;
use std::time::{Duration, Instant};

// Simple test without a Tokio runtime, to just have an integration
// test of the pollcatch hooks on async-profiler without involving
// Tokio

fn main() {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(1) {
        async_profiler_agent::pollcatch::before_poll_hook();
        let mid = Instant::now();
        while mid.elapsed() < Duration::from_millis(10) {
            // spin, there will be a profiler sample here
        }
        async_profiler_agent::pollcatch::after_poll_hook();
    }
}
