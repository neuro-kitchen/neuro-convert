//! ③ Metadata: the session and the subject. Essentials first, the rest under "More details";
//! fields with issues are outlined and say why.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::date_picker::DatePicker;
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::Select;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, IconName, Sizable as _};
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Window};

use crate::domain::format::AgeUnit;
use crate::viewmodels::metadata::{Field, MetadataVm};
use crate::widgets::{Card, FormRow, MenuSelect, Muted, PickInput, SuggestInput};

pub struct MetadataView {
    vm: Entity<MetadataVm>,
    _vm: Subscription,
}

impl MetadataView {
    pub fn new(vm: Entity<MetadataVm>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&vm, |_, _, cx| cx.notify());
        Self { vm, _vm: sub }
    }

    /// A text field row (with suggestions when the field has any).
    fn field(&self, f: Field, cx: &mut Context<Self>) -> AnyElement {
        let vm = self.vm.read(cx);
        let Some((_, state)) = vm.fields.iter().find(|(x, _)| *x == f) else { return div().into_any_element() };
        let suggestions = vm.suggestions(f, cx);
        let control = if suggestions.is_empty() {
            Input::new(state).id(SharedString::from(f.id())).small().into_any_element()
        } else {
            let handle = self.vm.clone();
            let pick = move |v, window: &mut Window, cx: &mut gpui_kit::App| handle.update(cx, |vm, cx| vm.pick(f, v, window, cx));
            // A list field adds what is picked, so it stays a text box; one value is a dropdown
            if f == Field::Experimenters { SuggestInput::new(f.id(), state, suggestions, pick).into_any_element() } else { PickInput::new(f.id(), state, suggestions, pick).into_any_element() }
        };
        FormRow::new(f.label(), control).required(f.required()).help(f.help()).issues(vm.issues(f.path(), cx)).into_any_element()
    }
}

impl Render for MetadataView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vm = self.vm.read(cx);
        if !vm.has_recording {
            return v_flex().id("metadata-form").test_support().size_full().p_6().child(Muted::new("Open a recording first.")).into_any_element();
        }
        let handle = self.vm.clone();

        // Start and zone
        let recorded = vm.recorded_start(cx);
        let reset = vm.start_overridden(cx).then(|| {
            let vm = handle.clone();
            Button::new("start-reset").ghost().xsmall().label("Use the recorded time").on_click(move |_, window, cx| vm.update(cx, |vm, cx| vm.reset_start(window, cx)))
        });
        let start = FormRow::new("Start time", h_flex().gap_2().child(div().w(px(300.)).child(DatePicker::new(&vm.start).small())).children(reset))
            .help(recorded.map_or_else(|| "Not recorded: set it".to_string(), |r| format!("Recorded: {r} (local time of the recording system)")))
            .issues(vm.issues("session.start_time", cx))
            .compact();
        let zone = FormRow::new(
            "Time zone",
            div().w(px(420.)).child(Select::new(&vm.zone).id("field-Timezone").small().placeholder("Where (when) the recording was made").search_placeholder("Search offset or city")),
        )
        .required(true)
        .help("Recording systems store local time; NWB needs its offset from UTC")
        .issues(vm.issues("session.timezone", cx))
        .compact();

        // Sex and age
        let sex = vm.sex(cx);
        let mut sex_buttons = h_flex().gap_1();
        for (code, label) in [("M", "Male"), ("F", "Female"), ("U", "Unknown"), ("O", "Other")] {
            let vm = handle.clone();
            let b = Button::new(SharedString::from(format!("sex-{code}"))).small().label(label);
            let b = if sex.as_deref() == Some(code) { b.primary() } else { b.outline() };
            sex_buttons = sex_buttons.child(b.on_click(move |_, _, cx| vm.update(cx, |vm, cx| vm.set_sex(code, cx))));
        }
        let current = vm.age_unit;
        let unit_select = {
            let vm = handle.clone();
            MenuSelect::new("age-unit", current.label(), AgeUnit::ALL.iter().map(|u| SharedString::from(u.label())).collect(), AgeUnit::ALL.iter().position(|u| *u == current), move |i, _, cx| {
                vm.update(cx, |vm, cx| vm.set_age_unit(AgeUnit::ALL[i], cx))
            })
        };
        let mut age_issues = vm.issues("subject.age", cx);
        if let Some(e) = &vm.age_error {
            age_issues.insert(0, (true, e.clone()));
        }
        let age = FormRow::new("Age", h_flex().gap_2().child(div().w(px(120.)).child(Input::new(&vm.age).id("field-Age").small())).child(unit_select))
            .help("At the time of the session")
            .issues(age_issues)
            .compact();

        let dandi = vm.dandi(cx);
        let dandi_switch = {
            let vm = handle.clone();
            Switch::new("dandi").checked(dandi).label("I'll upload to DANDI").on_click(move |on, _, cx| vm.update(cx, |vm, cx| vm.set_dandi(*on, cx)))
        };
        let sex_issues = vm.issues("subject.sex", cx);

        let more_open = vm.more;
        let more_toggle = {
            let vm = handle.clone();
            Button::new("more-details")
                .ghost()
                .small()
                .icon(if more_open { IconName::ChevronDown } else { IconName::ChevronRight })
                .label("More details: experiment, people, lab, keywords, strain")
                .on_click(move |_, _, cx| vm.update(cx, |vm, cx| vm.toggle_more(cx)))
        };

        let session = Card::new().title("Session").child(self.field(Field::Description, cx)).child(start).child(zone);
        let subject = Card::new()
            .title("Subject")
            .aside(dandi_switch)
            .child(self.field(Field::SubjectId, cx))
            .child(self.field(Field::Species, cx))
            .child(FormRow::new("Sex", sex_buttons).issues(sex_issues))
            .child(age);
        let mut body = v_flex().gap_4().w_full().max_w(px(860.)).child(session).child(subject).child(more_toggle);
        if more_open {
            let fields: Vec<AnyElement> = Field::MORE.iter().map(|f| self.field(*f, cx)).collect();
            body = body.child(Card::new().title("More details").children(fields));
        }
        // Centered column on wide windows
        v_flex().id("metadata-form").test_support().size_full().p_6().items_center().child(body).overflow_y_scrollbar().into_any_element()
    }
}
