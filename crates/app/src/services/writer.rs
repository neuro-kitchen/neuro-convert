//! Writing a store on its own thread. `Job::write` already spreads the copy over its worker
//! threads; this thread only drives it and forwards events. Progress events are sent as they come
//! (about twice a second); the receiver keeps only the latest one it finds queued.

use nc_convert::{Event, Job, Report};

use crate::domain::WriteTicket;

pub enum WriterMsg {
    Event(Event),
    /// The job comes back with the result.
    Done(Box<(Job, nc_convert::Result<Report>)>),
}

pub fn start(ticket: WriteTicket) -> async_channel::Receiver<WriterMsg> {
    let (tx, rx) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("nc-writer".into())
        .spawn(move || {
            let WriteTicket { job, output, options, cancel } = ticket;
            let events = tx.clone();
            let result = job.write(&output, &options, &cancel, &move |e| {
                let _ = events.send_blocking(WriterMsg::Event(e));
            });
            let _ = tx.send_blocking(WriterMsg::Done(Box::new((job, result))));
        })
        .expect("spawn writer thread");
    rx
}

/// Waits for the next message, then takes everything already queued: returns the newest event
/// (older progress is stale) and the result if the write ended.
pub async fn next(rx: &async_channel::Receiver<WriterMsg>) -> (Option<Event>, Option<Box<(Job, nc_convert::Result<Report>)>>) {
    let Ok(first) = rx.recv().await else { return (None, None) };
    let (mut event, mut done) = (None, None);
    for msg in std::iter::once(first).chain(std::iter::from_fn(|| rx.try_recv().ok())) {
        match msg {
            WriterMsg::Event(e) => event = Some(e),
            WriterMsg::Done(d) => done = Some(d),
        }
    }
    (event, done)
}
