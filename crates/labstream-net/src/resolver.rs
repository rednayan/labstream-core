//! The background resolver.
//!
//! A one-shot resolve sends a wave of queries, waits, and returns what it
//! found. A background resolver keeps sending waves and keeps a list that
//! grows when a stream appears and shrinks when one stops answering
//! (`src/resolver_impl.cpp:130-168`).
//!
//! An application uses this to show a live list of streams. `GetAllStreams`,
//! one of the example programs of liblsl, does exactly that.

use crate::info::StreamInfo;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long to wait between two waves.
///
/// `src/resolver_impl.cpp` adds `ContinuousResolveInterval` to
/// `MulticastMinRTT`, whose defaults are 0.5 and 0.5 (`src/api_config.cpp:307`
/// and `:311`).
fn wave_interval() -> Duration {
    let cfg = crate::config::get();
    Duration::from_secs_f64(cfg.continuous_resolve_interval + cfg.multicast_min_rtt)
}

struct Shared {
    /// One entry for each stream, with the clock reading of the last answer.
    found: Mutex<HashMap<String, (StreamInfo, f64)>>,
    stop: AtomicBool,
}

/// A resolver that keeps looking.
pub struct ContinuousResolver {
    shared: Arc<Shared>,
    /// How long a stream stays in the list after its last answer, in seconds.
    forget_after: f64,
}

impl ContinuousResolver {
    /// Start looking for streams that match a query.
    ///
    /// The thread stops when the resolver is dropped.
    pub fn new(query: &str, forget_after: f64) -> ContinuousResolver {
        let shared = Arc::new(Shared {
            found: Mutex::new(HashMap::new()),
            stop: AtomicBool::new(false),
        });
        let worker = Arc::clone(&shared);
        let q = query.to_string();
        std::thread::spawn(move || wave_loop(&q, worker));
        ContinuousResolver {
            shared,
            forget_after,
        }
    }

    /// The streams that answered lately.
    ///
    /// A stream whose last answer is older than `forget_after` is removed
    /// here, not by the thread. liblsl does the same
    /// (`src/resolver_impl.cpp:151-168`), so a caller that never asks keeps
    /// every entry.
    pub fn results(&self, max: usize) -> Vec<StreamInfo> {
        let now = crate::clock();
        let mut map = self.shared.found.lock().unwrap();
        map.retain(|_, (_, seen)| *seen >= now - self.forget_after);
        map.values().take(max).map(|(i, _)| i.clone()).collect()
    }

    /// How many streams the resolver holds, without removing any.
    pub fn len(&self) -> usize {
        self.shared.found.lock().unwrap().len()
    }

    /// True when the resolver holds no stream.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Drop for ContinuousResolver {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
    }
}

/// Send a wave, collect the answers, wait, and repeat.
fn wave_loop(query: &str, shared: Arc<Shared>) {
    let mut wave = match crate::inlet::QueryWave::open() {
        Ok(w) => w,
        Err(_) => return,
    };
    while !shared.stop.load(Ordering::SeqCst) {
        wave.send(query);
        let answers = wave.collect(Duration::from_millis(400), |_| false);
        if !answers.is_empty() {
            let now = crate::clock();
            let mut map = shared.found.lock().unwrap();
            for info in answers {
                map.insert(info.uid.clone(), (info, now));
            }
        }
        // Sleep in short steps, so a dropped resolver stops without a wait.
        let mut left = wave_interval();
        while left > Duration::ZERO && !shared.stop.load(Ordering::SeqCst) {
            let step = left.min(Duration::from_millis(100));
            std::thread::sleep(step);
            left -= step;
        }
    }
}
