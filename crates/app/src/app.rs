//! The root view: title bar, a slim toolbar, the step bar (① Source ② Contents ③ Metadata
//! ④ Review & convert) with issue counts, the current step, a footer (issues menu, Back / Next)
//! and the status bar. It builds the view models and screens, handles the app actions, and
//! re-renders only on the store / navigation events it shows.

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::stepper::{Stepper, StepperItem};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, IconName, Sizable as _, Theme, ThemeMode, TitleBar, WindowExt as _};
use gpui_kit::{
    div, px, AnyElement, AppContext as _, Context, Entity, ExternalPaths, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions, Render,
    Styled as _, Subscription, Window,
};
use nc_convert::core::Level;

use crate::actions::{Cancel, ChooseOutput, Convert, LoadMetadata, OpenRecording, OpenRecordingFile, Quit, SaveMetadata, ToggleTheme};
use crate::domain::steps::target_label;
use crate::domain::{AppEvent, Events, Step, Workspace};
use crate::state::Outcome;
use crate::store::Store;
use crate::viewmodels::{ContentsVm, ConvertVm, MetadataVm, NavVm, PlanVm, PreviewVm, SourceVm};
use crate::views::{ContentsView, MetadataView, PreviewView, ReviewView, SourceView};

pub struct NcApp {
    store: Entity<Store>,
    pub(crate) nav: Entity<NavVm>,
    /// Kept for tests, which drive the stream card through it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) contents_vm: Entity<ContentsVm>,
    source: Entity<SourceView>,
    contents: Entity<ContentsView>,
    metadata: Entity<MetadataView>,
    review: Entity<ReviewView>,
    /// Follow the system light / dark setting until the user picks one.
    follow_system_theme: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl NcApp {
    pub fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let nav = cx.new(|cx| NavVm::new(store.clone(), cx));
        let source_vm = cx.new(|cx| SourceVm::new(store.clone(), cx));
        let contents_vm = cx.new(|cx| ContentsVm::new(store.clone(), nav.clone(), window, cx));
        let metadata_vm = cx.new(|cx| MetadataVm::new(store.clone(), nav.clone(), window, cx));
        let preview_vm = cx.new(|cx| PreviewVm::new(store.clone(), cx));
        let plan_vm = cx.new(|cx| PlanVm::new(store.clone(), cx));
        let convert_vm = cx.new(|cx| ConvertVm::new(store.clone(), window, cx));
        let preview = cx.new(|cx| PreviewView::new(preview_vm, cx));
        let source = cx.new(|cx| SourceView::new(source_vm, cx));
        let contents = cx.new(|cx| ContentsView::new(contents_vm.clone(), preview, cx));
        let metadata = cx.new(|cx| MetadataView::new(metadata_vm, cx));
        let review = cx.new(|cx| ReviewView::new(plan_vm, convert_vm, nav.clone(), cx));

        // Theme: the saved choice, else follow the system
        let saved = store.read(cx).ws.settings.theme.clone();
        let follow_system_theme = saved.is_none();
        match saved.as_deref() {
            Some("dark") => Theme::change(ThemeMode::Dark, Some(window), cx),
            Some(_) => Theme::change(ThemeMode::Light, Some(window), cx),
            None => Theme::sync_system_appearance(Some(window), cx),
        }
        let subscriptions = vec![
            cx.observe_window_appearance(window, |this, window, cx| {
                if this.follow_system_theme {
                    Theme::sync_system_appearance(Some(window), cx);
                }
            }),
            cx.observe(&nav, |_, _, cx| cx.notify()),
            cx.subscribe_in(&store, window, |_, store, event: &AppEvent, window, cx| match event {
                // A conversion often ends while the window is in the background
                AppEvent::WriteFinished => {
                    if let Some(o) = &store.read(cx).ws.state.last_outcome {
                        let note = match o {
                            Outcome::Converted(p) => Notification::success(format!("Converted and verified: {}", p.display())),
                            Outcome::Cancelled => Notification::info("Conversion cancelled; the partial output was removed."),
                            Outcome::Failed(e) => Notification::error(format!("Conversion failed: {e}")),
                        };
                        window.push_notification(note, cx);
                    }
                    cx.notify();
                }
                AppEvent::Status | AppEvent::MetadataChanged | AppEvent::MetadataReloaded | AppEvent::PlanUpdated | AppEvent::RecordingOpened => cx.notify(),
                _ => {}
            }),
        ];
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self { store, nav, contents_vm, source, contents, metadata, review, follow_system_theme, focus, _subscriptions: subscriptions }
    }

    fn ws<'a>(&self, cx: &'a gpui_kit::App) -> &'a Workspace {
        &self.store.read(cx).ws
    }

    fn apply(&self, cx: &mut gpui_kit::App, change: impl FnOnce(&mut Workspace) -> Events) {
        self.store.update(cx, |s, cx| s.apply(cx, change));
    }

    /// Asks for a path with the platform dialog, then runs `then` with it.
    fn prompt_open(&self, directories: bool, prompt: &str, cx: &mut Context<Self>, then: impl FnOnce(&mut Store, PathBuf, &mut Context<Store>) + 'static) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: !directories, directories, multiple: false, prompt: Some(prompt.to_string().into()) });
        let store = self.store.clone();
        cx.spawn(async move |_, cx| match rx.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    store.update(cx, |s, cx| then(s, path, cx));
                }
            }
            Ok(Err(e)) => store.update(cx, |s, cx| {
                s.apply(cx, |ws| {
                    ws.state.message = Some(format!("File dialog failed: {e}"));
                    Events::from([AppEvent::Status])
                })
            }),
            _ => {}
        })
        .detach();
    }

    /// Asks for a new path (save dialog) next to `near`, suggesting `name`.
    fn prompt_save(&self, near: Option<PathBuf>, name: &str, cx: &mut Context<Self>, then: impl FnOnce(&mut Workspace, PathBuf) -> Events + 'static) {
        let dir = near.as_deref().and_then(|p| p.parent()).map(|p| p.to_path_buf()).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let rx = cx.prompt_for_new_path(&dir, Some(name));
        let store = self.store.clone();
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(path))) = rx.await {
                store.update(cx, |s, cx| s.apply(cx, |ws| then(ws, path)));
            }
        })
        .detach();
    }

    fn open_recording(&mut self, _: &OpenRecording, _: &mut Window, cx: &mut Context<Self>) {
        if self.ws(cx).state.can_open() {
            self.prompt_open(true, "Open recording", cx, |s, path, cx| s.open(path, None, cx));
        }
    }

    fn open_recording_file(&mut self, _: &OpenRecordingFile, _: &mut Window, cx: &mut Context<Self>) {
        if self.ws(cx).state.can_open() {
            self.prompt_open(false, "Open recording", cx, |s, path, cx| s.open(path, None, cx));
        }
    }

    fn load_metadata(&mut self, _: &LoadMetadata, _: &mut Window, cx: &mut Context<Self>) {
        if self.ws(cx).state.can_edit_metadata() {
            self.prompt_open(false, "Load metadata", cx, |s, path, cx| s.apply(cx, |ws| ws.load_metadata(&path)));
        }
    }

    fn save_metadata(&mut self, _: &SaveMetadata, _: &mut Window, cx: &mut Context<Self>) {
        let ws = self.ws(cx);
        if !ws.state.can_edit_metadata() {
            return;
        }
        let near = ws.metadata_path.clone().or_else(|| ws.state.source.clone());
        let name = ws.state.source.as_ref().and_then(|p| p.file_stem()).map_or_else(|| "metadata.yaml".to_string(), |s| format!("{}.yaml", s.to_string_lossy()));
        self.prompt_save(near, &name, cx, |ws, path| ws.save_metadata(&path));
    }

    fn choose_output(&mut self, _: &ChooseOutput, _: &mut Window, cx: &mut Context<Self>) {
        let near = self.ws(cx).output.clone();
        let name = near.as_ref().and_then(|p| p.file_name()).map_or_else(|| "recording.nwb.zarr".into(), |n| n.to_string_lossy().into_owned());
        self.prompt_save(near, &name, cx, |ws, path| ws.set_output(Some(path)));
    }

    fn convert(&mut self, _: &Convert, _: &mut Window, cx: &mut Context<Self>) {
        // Converting is done from the Review step: show it (progress and result are there)
        self.nav.update(cx, |nav, cx| nav.go(Step::Review, cx));
        self.store.update(cx, |s, cx| s.convert(cx));
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        self.apply(cx, Workspace::cancel);
    }

    fn toggle_theme(&mut self, _: &ToggleTheme, window: &mut Window, cx: &mut Context<Self>) {
        self.follow_system_theme = false;
        let dark = !cx.theme().is_dark();
        Theme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, Some(window), cx);
        self.store.update(cx, |s, _| s.ws.set_theme(dark));
        cx.notify();
    }

    /// Window title bar: app name and open recording; window controls come from `TitleBar`.
    fn title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let ws = self.ws(cx);
        let source = ws.state.source.as_ref().and_then(|p| p.file_name()).map(|n| format!(" — {}", n.to_string_lossy()));
        let dirty = if ws.metadata_dirty { " •" } else { "" };
        TitleBar::new().child(div().text_sm().text_color(cx.theme().foreground).child(format!("neuro-convert{}{dirty}", source.unwrap_or_default())))
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let s = &self.ws(cx).state;
        let (can_open, can_edit) = (s.can_open(), s.can_edit_metadata());
        let dark = cx.theme().is_dark();
        h_flex()
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("open")
                    .ghost()
                    .icon(IconName::FolderOpen)
                    .label("Open recording")
                    .tooltip("Open a recording folder (Ctrl+O); a single file with Ctrl+Shift+O")
                    .disabled(!can_open)
                    .on_click(cx.listener(|this, _, window, cx| this.open_recording(&OpenRecording, window, cx))),
            )
            .child(
                Button::new("load")
                    .ghost()
                    .icon(IconName::FileText)
                    .label("Load metadata")
                    .tooltip("Load a metadata YAML (Ctrl+L)")
                    .disabled(!can_edit)
                    .on_click(cx.listener(|this, _, window, cx| this.load_metadata(&LoadMetadata, window, cx))),
            )
            .child(
                Button::new("save")
                    .ghost()
                    .icon(IconName::File)
                    .label("Save metadata")
                    .tooltip("Save the metadata as YAML to reuse it (Ctrl+S)")
                    .disabled(!can_edit)
                    .on_click(cx.listener(|this, _, window, cx| this.save_metadata(&SaveMetadata, window, cx))),
            )
            .child(div().flex_1())
            .child(
                Button::new("theme")
                    .ghost()
                    .icon(if dark { IconName::Sun } else { IconName::Moon })
                    .accessibility_label(if dark { "Use the light theme" } else { "Use the dark theme" })
                    .tooltip(if dark { "Light theme" } else { "Dark theme" })
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_theme(&ToggleTheme, window, cx))),
            )
    }

    fn steps(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let nav = self.nav.read(cx);
        let counts = nav.counts(cx);
        let items: Vec<StepperItem> = Step::ALL
            .iter()
            .map(|step| {
                let (errors, warnings) = counts[step.index()];
                let badge = match (errors, warnings) {
                    (0, 0) => None,
                    (0, w) => Some(Tag::warning().xsmall().child(w.to_string())),
                    (e, _) => Some(Tag::danger().xsmall().child(e.to_string())),
                };
                StepperItem::new().disabled(!nav.enabled(*step, cx)).child(h_flex().gap_1p5().child(step.title()).children(badge))
            })
            .collect();
        let handle = self.nav.clone();
        div().px_6().py_3().border_b_1().border_color(cx.theme().border).child(
            Stepper::new("steps").small().selected_index(nav.step.index()).items(items).on_click(move |ix, _, cx| handle.update(cx, |nav, cx| nav.go(Step::ALL[*ix], cx))),
        )
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let nav = self.nav.read(cx);
        let step = nav.step;
        let issues = nav.issues(cx);
        let errors = issues.iter().filter(|i| i.level == Level::Error).count();
        let issues_menu = (!issues.is_empty()).then(|| {
            let label = format!("{errors} errors, {} warnings", issues.len() - errors);
            let handle = self.nav.clone();
            let b = Button::new("issues-menu").small().icon(IconName::TriangleAlert).label(label).dropdown_caret(true);
            let b = if errors > 0 { b.danger() } else { b.warning() };
            b.dropdown_menu(move |menu, _, _| {
                let mut menu = menu.scrollable(true).max_h(px(400.)).max_w(px(560.));
                for i in &issues {
                    let place = i.target.as_ref().map_or_else(|| "Review".to_string(), target_label);
                    let text = format!("{} — {}: {}", if i.level == Level::Error { "Error" } else { "Warning" }, place, i.message);
                    let (handle, target) = (handle.clone(), i.target.clone());
                    menu = menu.item(PopupMenuItem::new(text).on_click(move |_, _, cx| {
                        handle.update(cx, |nav, cx| match &target {
                            Some(t) => nav.reveal(t.clone(), cx),
                            None => nav.go(Step::Review, cx),
                        })
                    }));
                }
                menu
            })
        });
        let back = step.previous().map(|p| {
            let handle = self.nav.clone();
            Button::new("step-back").outline().icon(IconName::ChevronLeft).label(format!("Back: {}", p.title())).on_click(move |_, _, cx| handle.update(cx, |nav, cx| nav.go(p, cx)))
        });
        let next = step.next().map(|n| {
            let handle = self.nav.clone();
            Button::new("step-next").primary().label(format!("Next: {}", n.title())).icon(IconName::ChevronRight).disabled(!nav.enabled(n, cx)).on_click(move |_, _, cx| handle.update(cx, |nav, cx| nav.go(n, cx)))
        });
        h_flex().gap_2().px_6().py_2().border_t_1().border_color(cx.theme().border).children(issues_menu).child(div().flex_1()).children(back).children(next)
    }

    fn status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let s = &self.ws(cx).state;
        StatusBar::new().left(div().text_xs().child(s.phase_label())).child(div().text_xs().text_color(cx.theme().muted_foreground).child(s.message.clone().unwrap_or_default()))
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.nav.read(cx).step {
            Step::Source => self.source.clone().into_any_element(),
            Step::Contents => self.contents.clone().into_any_element(),
            Step::Metadata => self.metadata.clone().into_any_element(),
            Step::Review => self.review.clone().into_any_element(),
        }
    }
}

impl Render for NcApp {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.clone();
        v_flex()
            .id("nc-app")
            .track_focus(&self.focus)
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(Self::open_recording))
            .on_action(cx.listener(Self::open_recording_file))
            .on_action(cx.listener(Self::load_metadata))
            .on_action(cx.listener(Self::save_metadata))
            .on_action(cx.listener(Self::choose_output))
            .on_action(cx.listener(Self::convert))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::toggle_theme))
            .on_action(|_: &Quit, _, cx| cx.quit())
            // A folder or file dropped anywhere opens it
            .on_drop(move |paths: &ExternalPaths, _, cx| {
                if let Some(p) = paths.paths().first().cloned() {
                    store.update(cx, |s, cx| s.open(p, None, cx));
                }
            })
            .child(self.title_bar(cx))
            .child(self.toolbar(cx))
            .child(self.steps(cx))
            .child(div().flex_1().min_h_0().child(self.body(cx)))
            .child(self.footer(cx))
            .child(self.status_bar(cx))
    }
}

/// Headless UI tests with the fake reader (`domain::workspace::tests::Fake`): real windows and
/// hit testing, no pixels.
#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{px, size, AnyWindowHandle, AppContext as _, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions};
    use nc_convert::core::{ItemKind, StreamType, Target};
    use nc_convert::Registry;

    use super::NcApp;
    use crate::domain::workspace::tests::Fake;
    use crate::domain::{Step, Workspace};
    use crate::settings::Settings;
    use crate::state::{Outcome, Phase};
    use crate::store::Store;

    fn open(cx: &mut TestAppContext, output_dir: &Path) -> (AnyWindowHandle, Entity<Store>, Entity<NcApp>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::actions::bind_keys(cx);
            let settings = Settings { output_dir: Some(output_dir.to_path_buf()), gzip: None, ..Settings::default() };
            let store = cx.new(|_| Store::new(Workspace::new(Arc::new(Registry::empty().with(Fake)), settings, None)));
            let bounds = Bounds { origin: Point::default(), size: size(px(1400.), px(900.)) };
            let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
            let s = store.clone();
            let (window, app) = gpui_kit::open_window(options, cx, move |window, cx| cx.new(|cx| NcApp::new(s, window, cx))).expect("open test window");
            (window, store, app)
        })
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/app-ui-test").join(name);
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// Waits (real time) for work on a service thread to come back to the UI.
    fn wait_until(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
        cx.executor().allow_parking();
        for _ in 0..500 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out");
    }

    fn phase(cx: &mut TestAppContext, store: &Entity<Store>) -> Phase {
        cx.update(|cx| store.read(cx).ws.state.phase.clone())
    }

    fn go(cx: &mut TestAppContext, app: &Entity<NcApp>, step: Step) {
        let nav = cx.update(|cx| app.read(cx).nav.clone());
        cx.update(|cx| nav.update(cx, |nav, cx| nav.go(step, cx)));
        cx.run_until_parked();
    }

    fn step(cx: &mut TestAppContext, app: &Entity<NcApp>) -> Step {
        cx.update(|cx| app.read(cx).nav.read(cx).step)
    }

    /// Opens `path` through the store (as the dialogs do) and waits for the open thread.
    fn open_recording(cx: &mut TestAppContext, store: &Entity<Store>, path: &str) {
        let path = Path::new(path).to_path_buf();
        cx.update(|cx| store.update(cx, |s, cx| s.open(path, None, cx)));
        wait_until(cx, |cx| !matches!(phase(cx, store), Phase::Opening(_)));
    }

    fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn steps_follow_the_recording(cx: &mut TestAppContext) {
        let (window, store, app) = open(cx, &scratch("layout"));
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let s = window.find("source-step").bounds().size;
            assert!(s.width > px(400.) && s.height > px(200.), "{s:?}");
            // Nothing open: no way forward
            window.click("step-next", cx);
        })
        .unwrap();
        assert_eq!(step(cx, &app), Step::Source);
        open_recording(cx, &store, "session.fake");
        assert_eq!(step(cx, &app), Step::Contents, "opening moves on to the contents");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("include-s/Wav1").is_some() && window.try_find("include-s/Temp").is_some());
            assert!(window.find("contents-step").bounds().size.width > px(800.));
        })
        .unwrap();
        click(cx, window, "step-next");
        assert_eq!(step(cx, &app), Step::Metadata);
        click(cx, window, "step-next");
        assert_eq!(step(cx, &app), Step::Review);
        click(cx, window, "step-back");
        assert_eq!(step(cx, &app), Step::Metadata);
    }

    #[gpui_kit::test]
    fn tree_form_and_issue_links_drive_the_plan(cx: &mut TestAppContext) {
        let (window, store, app) = open(cx, &scratch("plan"));
        open_recording(cx, &store, "session.fake");
        // Leave out Temp from the tree
        click(cx, window, "include-s/Temp");
        cx.update(|cx| {
            let ws = &store.read(cx).ws;
            assert!(!ws.included(ItemKind::Stream, "Temp"));
            assert_eq!(ws.plan.as_ref().unwrap().skipped, vec!["stream Temp".to_string()]);
        });
        // Review lists the errors; the link of the first one opens the metadata step
        go(cx, &app, Step::Review);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click(("review-issues-fix", 0usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(step(cx, &app), Step::Metadata);
        // Type the description
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("field-Description", cx);
            window.input("Synthetic test session", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let ws = &store.read(cx).ws;
            assert_eq!(ws.meta.session.description.as_deref(), Some("Synthetic test session"));
            assert!(ws.metadata_dirty);
            assert!(!ws.included(ItemKind::Stream, "Temp"), "form edits keep the include flags");
            let issues = &ws.plan.as_ref().unwrap().issues;
            assert!(issues.iter().any(|i| i.target == Some(Target::Field("session.timezone".into()))), "time zone still missing");
        });
    }

    #[gpui_kit::test]
    fn stream_card_sets_signal_kind_and_group(cx: &mut TestAppContext) {
        let (window, store, app) = open(cx, &scratch("card"));
        open_recording(cx, &store, "session.fake");
        // The first stream is selected; make it neural
        click(cx, window, "kind-neural");
        let stream_error = |cx: &mut TestAppContext| {
            cx.update(|cx| store.read(cx).ws.plan.as_ref().unwrap().issues.iter().any(|i| i.target == Some(Target::Stream("Wav1".into())) && i.level == nc_convert::core::Level::Error))
        };
        cx.update(|cx| assert_eq!(store.read(cx).ws.meta.stream("Wav1").kind, Some(StreamType::Electrical)));
        assert!(stream_error(cx), "neural without electrodes is an error");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("group-select").is_some());
        })
        .unwrap();
        // A new group fixes it
        let contents = cx.update(|cx| app.read(cx).contents_vm.clone());
        cx.update_window(window, |_, window, cx| contents.update(cx, |vm, cx| vm.new_group(window, cx))).unwrap();
        cx.run_until_parked();
        assert!(!stream_error(cx));
        cx.update(|cx| {
            let m = &store.read(cx).ws.meta;
            assert_eq!(m.stream("Wav1").electrode_group.as_deref(), Some("group1"));
            assert_eq!(m.electrode_groups[0].name, "group1");
        });
        // The group's location is typed in the card
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("group-location", cx);
            window.input("M1", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(store.read(cx).ws.meta.electrode_groups[0].location, "M1"));
    }

    #[gpui_kit::test]
    fn containers_offer_a_choice(cx: &mut TestAppContext) {
        let (window, store, app) = open(cx, &scratch("tank"));
        open_recording(cx, &store, "tank.fake");
        assert_eq!(step(cx, &app), Step::Source);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(store.read(cx).ws.pending.as_ref().unwrap().containers, vec!["b1", "b2"]);
            window.click(("container", 1usize), cx);
        })
        .unwrap();
        wait_until(cx, |cx| phase(cx, &store) == Phase::Ready);
        cx.update(|cx| assert!(store.read(cx).ws.pending.is_none()));
        assert_eq!(step(cx, &app), Step::Contents);
    }

    #[gpui_kit::test]
    fn convert_runs_in_the_background_and_reports(cx: &mut TestAppContext) {
        let dir = scratch("convert");
        let (window, store, app) = open(cx, &dir);
        open_recording(cx, &store, "session.fake");
        cx.update(|cx| {
            store.update(cx, |s, cx| {
                let mut meta = s.ws.meta.clone();
                meta.session.description = Some("ui convert".into());
                meta.session.timezone = Some("Z".into());
                s.apply(cx, |ws| ws.set_meta(meta));
            });
        });
        go(cx, &app, Step::Review);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("convert-now", cx);
        })
        .unwrap();
        // The button dispatches the Convert action (handled after this update)
        cx.run_until_parked();
        assert_ne!(phase(cx, &store), Phase::Ready, "conversion started");
        // The writer thread's messages are awaited; no timer
        wait_until(cx, |cx| phase(cx, &store) != Phase::Writing);
        cx.update(|cx| {
            let ws = &store.read(cx).ws;
            assert!(matches!(ws.state.last_outcome, Some(Outcome::Converted(_))), "{:?}", ws.state.last_outcome);
            assert!(ws.report.as_ref().unwrap().output.join("zarr.json").exists());
            assert!(ws.job.is_some());
        });
    }

    #[gpui_kit::test]
    fn theme_toggle_is_remembered(cx: &mut TestAppContext) {
        let (window, store, _) = open(cx, &scratch("theme"));
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let dark = cx.theme().is_dark();
            window.click("theme", cx);
            assert_eq!(cx.theme().is_dark(), !dark);
            assert_eq!(store.read(cx).ws.settings.theme.as_deref(), Some(if dark { "light" } else { "dark" }));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn preview_follows_the_selected_stream(cx: &mut TestAppContext) {
        let (window, store, app) = open(cx, &scratch("preview"));
        open_recording(cx, &store, "session.fake");
        assert_eq!(cx.update(|cx| store.read(cx).ws.preview.clone()), Some("Wav1".into()));
        wait_until(cx, |cx| {
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.try_find("preview-view").is_some_and(|v| v.bounds().size.height > px(200.))
            })
            .unwrap()
        });
        // Panning right is possible on a 2 s recording with a 1 s window
        click(cx, window, "pan-right");
        // Selecting the other stream in the tree switches the preview
        let contents = cx.update(|cx| app.read(cx).contents_vm.clone());
        cx.update_window(window, |_, window, cx| contents.update(cx, |vm, cx| vm.select_id("s/Temp", window, cx))).unwrap();
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| store.read(cx).ws.preview.clone()), Some("Temp".into()));
    }
}
