//! The NWB plan screen: display rows built once per plan (not per frame) by [`plan_rows`].
//! Listens to: PlanUpdated, RecordingOpened, WriteFinished. Keeps showing the last plan while the
//! job is away writing.

use gpui_kit::{Context, Entity, SharedString, Subscription};
use nc_convert::core::{Level, Session};
use nc_convert::nwb::NwbPlan;

use crate::domain::AppEvent;
use crate::store::Store;
use crate::widgets::{IssueRow, PathRow};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlanRows {
    /// `2 series · 1 event table → name.nwb.zarr`.
    pub summary: String,
    pub errors: usize,
    pub warnings: usize,
    /// Errors first.
    pub issues: Vec<IssueRow>,
    pub file: Vec<String>,
    pub sections: Vec<(String, Vec<PathRow>)>,
    pub skipped: Vec<String>,
}

impl PartialEq for PathRow {
    fn eq(&self, o: &Self) -> bool {
        (&self.path, &self.source, &self.detail) == (&o.path, &o.source, &o.detail)
    }
}

impl std::fmt::Debug for PathRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ← {} ({})", self.path, self.source, self.detail)
    }
}

pub fn plan_rows(plan: &NwbPlan, s: &Session) -> PlanRows {
    let row = |path: String, source: &str, detail: String| PathRow { path: path.into(), source: SharedString::from(source.to_string()), detail: detail.into() };
    let mut issues: Vec<IssueRow> = plan.issues.iter().map(|i| IssueRow { error: i.level == Level::Error, text: i.message.clone().into(), target: i.target.clone() }).collect();
    issues.sort_by_key(|i| !i.error);
    let errors = issues.iter().filter(|i| i.error).count();
    let f = &plan.file;
    let file = vec![
        format!("identifier {}", f.identifier),
        format!("start {}", if f.start_time.is_empty() { "?" } else { &f.start_time }),
        format!("subject {} ({})", plan.subject.id.as_deref().unwrap_or("?"), plan.subject.species.as_deref().unwrap_or("species?")),
    ];
    let mut acquisition: Vec<PathRow> = plan
        .series
        .iter()
        .map(|p| {
            let i = s.recordings[p.recording].info();
            let kind = if p.electrodes.is_some() { "Electrical" } else { "TimeSeries" };
            row(format!("/acquisition/{}", p.name), &p.source, format!("{kind} · {} ch · {}", i.channel_count(), p.unit))
        })
        .collect();
    acquisition.extend(plan.events.iter().filter(|e| !e.table).map(|e| row(format!("/acquisition/{}", e.name), &e.source, "TimeSeries (scalars)".into())));
    acquisition.extend(plan.snippets.iter().map(|p| row(format!("/acquisition/{}_ch*", p.name), &p.source, format!("SpikeEventSeries · {} ch", p.rows.len()))));
    let events = plan.events.iter().filter(|e| e.table).map(|e| row(format!("/events/{}", e.name), &e.source, format!("{} rows", s.events[e.event].len()))).collect();
    let mut other: Vec<PathRow> = plan
        .groups
        .iter()
        .map(|g| row(format!("electrodes: {}", g.name), &g.device, format!("{} · {}", s.electrodes.iter().filter(|e| e.group == g.name).count(), g.location)))
        .collect();
    other.extend(plan.tables.iter().map(|t| row(format!("/analysis/{}", t.name), &s.tables[t.table].name, "DynamicTable".into())));
    let count = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut parts = vec![count(plan.series.len() + plan.events.iter().filter(|e| !e.table).count(), "series", "series")];
    let tables = plan.events.iter().filter(|e| e.table).count();
    if tables > 0 {
        parts.push(count(tables, "event table", "event tables"));
    }
    if !plan.tables.is_empty() {
        parts.push(count(plan.tables.len(), "table", "tables"));
    }
    if !plan.snippets.is_empty() {
        parts.push(count(plan.snippets.len(), "snippet store", "snippet stores"));
    }
    PlanRows {
        summary: parts.join(" · "),
        errors,
        warnings: issues.len() - errors,
        issues,
        file,
        sections: vec![("Acquisition".into(), acquisition), ("Events".into(), events), ("Electrodes and tables".into(), other)],
        skipped: plan.skipped.clone(),
    }
}

pub struct PlanVm {
    pub rows: Option<PlanRows>,
    _store: Subscription,
}

impl PlanVm {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let sub = cx.subscribe(&store, |this, store, event: &AppEvent, cx| {
            if matches!(event, AppEvent::PlanUpdated | AppEvent::RecordingOpened | AppEvent::WriteFinished) {
                let ws = &store.read(cx).ws;
                if let (Some(plan), Some(s)) = (&ws.plan, ws.session()) {
                    let rows = plan_rows(plan, s);
                    if this.rows.as_ref() != Some(&rows) {
                        this.rows = Some(rows);
                        cx.notify();
                    }
                }
            }
        });
        Self { rows: None, _store: sub }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workspace::tests::{open, scratch, workspace};
    use nc_convert::core::ItemKind;

    #[test]
    fn test_rows_follow_the_plan() {
        let mut ws = workspace(&scratch("plan-vm"));
        open(&mut ws, "session.fake");
        let rows = plan_rows(ws.plan.as_ref().unwrap(), ws.session().unwrap());
        assert!(rows.errors >= 1 && rows.issues[0].error, "errors first: {:?}", rows.issues);
        assert_eq!(rows.sections[0].1.len(), 2, "two acquisition series");
        assert_eq!(rows.summary, "2 series · 1 event table");
        ws.set_included(ItemKind::Stream, &["Temp".into()], false);
        let rows = plan_rows(ws.plan.as_ref().unwrap(), ws.session().unwrap());
        assert_eq!((rows.sections[0].1.len(), rows.skipped.clone()), (1, vec!["stream Temp".to_string()]));
    }
}
