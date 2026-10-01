//! Long-running, blocking work (reading recordings, writing stores, preview sampling) runs on
//! dedicated named threads, never on GPUI's executors: they stay free for the UI. Results come
//! back over channels the UI awaits (no polling).

pub mod sampler;
pub mod writer;

/// Runs `work` on a new named thread; the returned channel yields its result once.
pub fn run<T: Send + 'static>(name: &str, work: impl FnOnce() -> T + Send + 'static) -> async_channel::Receiver<T> {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name(format!("nc-{name}"))
        .spawn(move || {
            let _ = tx.send_blocking(work());
        })
        .expect("spawn worker thread");
    rx
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_run_returns_on_its_own_thread() {
        let rx = super::run("test", || std::thread::current().name().map(str::to_string));
        assert_eq!(rx.recv_blocking().unwrap().as_deref(), Some("nc-test"));
    }
}
