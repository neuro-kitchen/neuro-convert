//! Preview sampling on one long-lived thread. Only the newest request matters: a new request
//! cancels the one in progress and replaces any waiting one, so panning fast never queues work.
//! Results are min / max envelopes (`nc_convert::preview::envelope`).

use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use nc_convert::core::Recording;

/// What to sample.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub recording: String,
    pub samples: Range<u64>,
    /// Channel indices, in lane order.
    pub channels: Vec<usize>,
    pub columns: usize,
}

pub type Lanes = Arc<Vec<Vec<(f32, f32)>>>;
pub type Reply = async_channel::Sender<(Key, nc_convert::Result<Lanes>)>;

struct Request {
    key: Key,
    recording: Arc<dyn Recording>,
    reply: Reply,
    stop: Arc<AtomicBool>,
}

#[derive(Default)]
struct Slot {
    next: Option<Request>,
    shutdown: bool,
}

pub struct Sampler {
    slot: Arc<(Mutex<Slot>, Condvar)>,
    /// Stop flag of the request being worked on.
    current: Mutex<Option<Arc<AtomicBool>>>,
}

impl Sampler {
    pub fn new() -> Self {
        let slot: Arc<(Mutex<Slot>, Condvar)> = Arc::default();
        let worker = slot.clone();
        std::thread::Builder::new()
            .name("nc-sampler".into())
            .spawn(move || {
                let (lock, wake) = &*worker;
                loop {
                    let request = {
                        let mut s = lock.lock().unwrap();
                        while s.next.is_none() && !s.shutdown {
                            s = wake.wait(s).unwrap();
                        }
                        if s.shutdown {
                            return;
                        }
                        s.next.take().unwrap()
                    };
                    if request.stop.load(Ordering::Relaxed) {
                        continue;
                    }
                    let k = &request.key;
                    let result = nc_convert::preview::envelope(request.recording.as_ref(), &k.channels, k.samples.clone(), k.columns, Some(&request.stop)).map(Arc::new);
                    if !matches!(result, Err(nc_convert::Error::Cancelled)) {
                        let _ = request.reply.send_blocking((request.key, result));
                    }
                }
            })
            .expect("spawn sampler thread");
        Self { slot, current: Mutex::new(None) }
    }

    /// Asks for `key`; the answer arrives on `reply`. Cancels whatever was asked before.
    pub fn request(&self, key: Key, recording: Arc<dyn Recording>, reply: Reply) {
        let stop = Arc::new(AtomicBool::new(false));
        if let Some(previous) = self.current.lock().unwrap().replace(stop.clone()) {
            previous.store(true, Ordering::Relaxed);
        }
        let (lock, wake) = &*self.slot;
        lock.lock().unwrap().next = Some(Request { key, recording, reply, stop });
        wake.notify_one();
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        if let Some(c) = self.current.lock().unwrap().take() {
            c.store(true, Ordering::Relaxed);
        }
        let (lock, wake) = &*self.slot;
        lock.lock().unwrap().shutdown = true;
        wake.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nc_convert::core::MemoryRecording;

    #[test]
    fn test_newest_request_wins() {
        let rec: Arc<dyn Recording> = Arc::new(MemoryRecording::new("r", (0..20_000).map(|v| v as f32).collect(), 2, 1000.0, "V").unwrap());
        let sampler = Sampler::new();
        let (tx, rx) = async_channel::unbounded();
        let key = |end| Key { recording: "r".into(), samples: 0..end, channels: vec![0, 1], columns: 10 };
        // Several quick requests: at least the last one is answered, and answers are correct
        for end in [10_000, 9_000, 8_000] {
            sampler.request(key(end), rec.clone(), tx.clone());
        }
        let mut last = None;
        while let Ok((k, lanes)) = rx.recv_blocking() {
            let lanes = lanes.unwrap();
            assert_eq!((lanes.len(), lanes[0].len()), (2, 10));
            let end = k.samples.end;
            assert_eq!(lanes[0][9].1, (end - 1) as f32, "channel 0 max of the last column");
            last = Some(end);
            if end == 8_000 {
                break;
            }
        }
        assert_eq!(last, Some(8_000));
    }
}
