//! The GPUI side of the [`Workspace`]: one entity that applies changes and emits their
//! [`AppEvent`]s (never a blanket `notify`), and connects the background services. View models
//! subscribe to the events they show.

use std::path::PathBuf;

use gpui_kit::{Context, EventEmitter};
use nc_convert::Job;

use crate::domain::{AppEvent, Events, Workspace};
use crate::services;

pub struct Store {
    pub ws: Workspace,
}

impl EventEmitter<AppEvent> for Store {}

impl Store {
    pub fn new(ws: Workspace) -> Self {
        Self { ws }
    }

    fn emit(&self, events: Events, cx: &mut Context<Self>) {
        for e in events.iter() {
            cx.emit(e);
        }
    }

    /// Applies a synchronous change and announces it.
    pub fn apply(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Workspace) -> Events) {
        let events = change(&mut self.ws);
        self.emit(events, cx);
    }

    /// Reads `path` on its own thread (containers first offer a choice).
    pub fn open(&mut self, path: PathBuf, block: Option<String>, cx: &mut Context<Self>) {
        let mut events = Events::default();
        let options = self.ws.begin_open(path.clone(), block, &mut events);
        self.emit(events, cx);
        let Some(options) = options else { return };
        let registry = self.ws.registry();
        let rx = services::run("open", move || Job::open(&registry, &path, &options));
        cx.spawn(async move |this, cx| {
            if let Ok(result) = rx.recv().await {
                let _ = this.update(cx, |store, cx| {
                    let events = store.ws.finish_open(result);
                    store.emit(events, cx);
                });
            }
        })
        .detach();
    }

    /// Writes the planned store on the writer thread; progress and the result come back as events.
    pub fn convert(&mut self, cx: &mut Context<Self>) {
        let mut events = Events::default();
        let ticket = self.ws.begin_write(&mut events);
        self.emit(events, cx);
        let Some(ticket) = ticket else { return };
        let rx = services::writer::start(ticket);
        cx.spawn(async move |this, cx| {
            loop {
                let (event, done) = services::writer::next(&rx).await;
                if event.is_none() && done.is_none() {
                    break;
                }
                let finished = done.is_some();
                let alive = this.update(cx, |store, cx| {
                    let mut events = Events::default();
                    if let Some(e) = event {
                        events.extend(store.ws.write_event(e));
                    }
                    if let Some(d) = done {
                        let (job, result) = *d;
                        events.extend(store.ws.finish_write(job, result));
                    }
                    store.emit(events, cx);
                });
                if finished || alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }
}
