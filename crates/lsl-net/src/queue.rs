//! A bounded sample queue that drops the oldest sample. SPEC.md 8.7.
//!
//! liblsl holds one of these per consumer on the outlet side
//! (`src/consumer_queue.h`), and one on the inlet side for samples that the
//! application has not pulled yet (`src/data_receiver.h:35`). Both drop the
//! oldest sample when they fill.
//!
//! The recorded behavior that this matches is in
//! `artifacts/behavior-liblsl.json`. A ring of 100 samples, fed 600 samples
//! faster than the reader consumed them, delivered sample 0 and then samples
//! 500 to 599. The writer never waited.

use lsl_wire::Sample;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// A queue that never blocks its writer.
#[derive(Debug)]
pub struct SampleQueue {
    inner: Mutex<VecDeque<Sample>>,
    room: Condvar,
    capacity: usize,
    dropped: AtomicU64,
    closed: Mutex<bool>,
}

impl SampleQueue {
    /// Build a queue that holds `capacity` samples.
    ///
    /// A capacity of zero would drop every sample, so the value floors at 1.
    /// liblsl floors the same way (`src/stream_info_impl.cpp:268`).
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        SampleQueue {
            inner: Mutex::new(VecDeque::with_capacity(capacity.min(4096))),
            room: Condvar::new(),
            capacity,
            dropped: AtomicU64::new(0),
            closed: Mutex::new(false),
        }
    }

    /// How many samples the queue holds when full.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// How many samples the queue has dropped.
    ///
    /// Nothing on the wire carries this number. SPEC.md 8.7 records that a
    /// reader cannot see a loss, so this exists for a local report only.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::SeqCst)
    }

    /// How many samples are waiting.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// True when no sample is waiting.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Add one sample. This never waits.
    ///
    /// A full queue loses its oldest sample. The writer carries on, so a slow
    /// reader never holds back the source. SPEC.md 8.7.
    pub fn push(&self, s: Sample) {
        let mut q = self.inner.lock().unwrap();
        if q.len() >= self.capacity {
            q.pop_front();
            self.dropped.fetch_add(1, Ordering::SeqCst);
        }
        q.push_back(s);
        drop(q);
        self.room.notify_one();
    }

    /// Take the oldest sample, waiting up to `timeout`.
    ///
    /// Returns `None` when the timeout passes with nothing waiting, or when the
    /// queue is closed and empty.
    pub fn pop(&self, timeout: Duration) -> Option<Sample> {
        let end = Instant::now() + timeout;
        let mut q = self.inner.lock().unwrap();
        loop {
            if let Some(s) = q.pop_front() {
                return Some(s);
            }
            if *self.closed.lock().unwrap() {
                return None;
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            let (guard, result) = self
                .room
                .wait_timeout(q, left.min(Duration::from_millis(100)))
                .unwrap();
            q = guard;
            if result.timed_out() && Instant::now() >= end {
                return q.pop_front();
            }
        }
    }

    /// Wake every reader and stop the queue.
    pub fn close(&self) {
        *self.closed.lock().unwrap() = true;
        self.room.notify_all();
    }

    /// True once [`SampleQueue::close`] has run.
    pub fn is_closed(&self) -> bool {
        *self.closed.lock().unwrap()
    }
}

/// A set of consumers that all receive the same sample.
///
/// liblsl calls this the send buffer (`src/send_buffer.h`). Each consumer holds
/// its own queue, so a slow consumer loses only its own samples. SPEC.md 8.8.
#[derive(Debug, Default)]
pub struct Fanout {
    consumers: Mutex<Vec<Arc<SampleQueue>>>,
}

impl Fanout {
    /// Build an empty set.
    pub fn new() -> Self {
        Fanout::default()
    }

    /// Add a consumer with its own queue.
    pub fn add(&self, capacity: usize) -> Arc<SampleQueue> {
        let q = Arc::new(SampleQueue::new(capacity));
        self.consumers.lock().unwrap().push(Arc::clone(&q));
        q
    }

    /// Remove a consumer.
    pub fn remove(&self, q: &Arc<SampleQueue>) {
        self.consumers
            .lock()
            .unwrap()
            .retain(|c| !Arc::ptr_eq(c, q));
    }

    /// How many consumers are connected.
    pub fn count(&self) -> usize {
        self.consumers.lock().unwrap().len()
    }

    /// Give one sample to every consumer.
    ///
    /// A consumer that has gone away is dropped from the set. The call never
    /// waits, whatever any consumer is doing.
    pub fn push(&self, s: &Sample) {
        let mut set = self.consumers.lock().unwrap();
        set.retain(|q| !q.is_closed());
        for q in set.iter() {
            q.push(s.clone());
        }
    }

    /// Close every consumer queue.
    pub fn close(&self) {
        for q in self.consumers.lock().unwrap().iter() {
            q.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsl_wire::Value;

    fn sample(i: i32) -> Sample {
        Sample {
            timestamp: i as f64,
            values: vec![Value::I32(i)],
        }
    }

    fn index(s: &Sample) -> i32 {
        match s.values[0] {
            Value::I32(v) => v,
            _ => panic!("wrong format"),
        }
    }

    #[test]
    fn a_full_queue_keeps_the_newest() {
        // The recorded shape from `artifacts/behavior-liblsl.json`, in small.
        let q = SampleQueue::new(100);
        for i in 0..600 {
            q.push(sample(i));
        }
        assert_eq!(q.len(), 100);
        assert_eq!(q.dropped(), 500);

        let first = q.pop(Duration::from_millis(10)).expect("a sample");
        assert_eq!(index(&first), 500, "the oldest surviving sample");

        let mut last = first;
        while let Some(s) = q.pop(Duration::from_millis(10)) {
            last = s;
        }
        assert_eq!(index(&last), 599, "the newest sample");
    }

    #[test]
    fn the_writer_never_waits() {
        // A queue of one, written a million times, must finish at once. A
        // design that waited for a reader would never return.
        let q = SampleQueue::new(1);
        let start = Instant::now();
        for i in 0..1_000_000 {
            q.push(sample(i));
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the writer waited"
        );
        assert_eq!(q.len(), 1);
        assert_eq!(index(&q.pop(Duration::from_millis(10)).unwrap()), 999_999);
    }

    #[test]
    fn a_capacity_of_zero_floors_at_one() {
        let q = SampleQueue::new(0);
        assert_eq!(q.capacity(), 1);
        q.push(sample(7));
        assert_eq!(index(&q.pop(Duration::from_millis(10)).unwrap()), 7);
    }

    #[test]
    fn an_empty_queue_reports_nothing_after_the_timeout() {
        let q = SampleQueue::new(4);
        let start = Instant::now();
        assert!(q.pop(Duration::from_millis(120)).is_none());
        assert!(start.elapsed() >= Duration::from_millis(100));
    }

    #[test]
    fn a_closed_queue_stops_a_reader_at_once() {
        let q = Arc::new(SampleQueue::new(4));
        let c = Arc::clone(&q);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            c.close();
        });
        let start = Instant::now();
        assert!(q.pop(Duration::from_secs(10)).is_none());
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "the reader waited out the timeout"
        );
    }

    #[test]
    fn a_reader_wakes_when_a_sample_arrives() {
        let q = Arc::new(SampleQueue::new(4));
        let c = Arc::clone(&q);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            c.push(sample(42));
        });
        let got = q.pop(Duration::from_secs(2)).expect("a sample");
        assert_eq!(index(&got), 42);
    }

    #[test]
    fn a_slow_consumer_never_holds_back_a_fast_one() {
        // SPEC.md 8.8. The fast queue is large and the slow queue is small.
        let f = Fanout::new();
        let fast = f.add(1000);
        let slow = f.add(10);
        for i in 0..500 {
            f.push(&sample(i));
        }
        assert_eq!(fast.len(), 500, "the fast consumer lost samples");
        assert_eq!(fast.dropped(), 0);
        assert_eq!(slow.len(), 10);
        assert_eq!(slow.dropped(), 490);
    }

    #[test]
    fn a_closed_consumer_leaves_the_set() {
        let f = Fanout::new();
        let a = f.add(10);
        let b = f.add(10);
        assert_eq!(f.count(), 2);
        b.close();
        f.push(&sample(1));
        assert_eq!(f.count(), 1, "a closed consumer must be removed");
        assert_eq!(a.len(), 1);
    }

    #[test]
    fn removing_a_consumer_stops_its_samples() {
        let f = Fanout::new();
        let a = f.add(10);
        f.push(&sample(1));
        f.remove(&a);
        f.push(&sample(2));
        assert_eq!(a.len(), 1, "a removed consumer must receive nothing more");
    }
}
