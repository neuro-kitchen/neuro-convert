# nc-app: neuro-convert desktop app

GPUI + gpui-kit front end over `nc_convert::Job`. Plan and milestones:
`.tasks/10-01-2026/02-nc-app/`.

```
cargo run -p nc-app          # binary: neuro-convert-app
```
`nc-app` is not in the workspace `default-members`, so `cargo build` / `cargo test` without `-p`
skip it (GPUI is a large build).

## Versions
`gpui-kit` is pinned exactly (`=0.7.0` in the workspace `Cargo.toml`); it pins `gpui-pre = 0.3.7`,
a weekly snapshot of Zed's GPUI whose API changes between releases. Update deliberately.

## System packages (Fedora 44)
Linking needs the development packages of:

| Library | Package |
|---|---|
| xcb | `libxcb-devel` |
| xkbcommon, xkbcommon-x11 | `libxkbcommon-devel`, `libxkbcommon-x11-devel` |
| fontconfig, freetype | `fontconfig-devel`, `freetype-devel` |
| Wayland | `wayland-devel` |

The X11 packages are needed **even on Wayland**: gpui-kit enables both GPUI Linux backends
(`gpui-pre-platform` features `x11` + `wayland`) and Cargo features cannot be turned off
downstream, so the X11 client is linked in. At start GPUI picks the backend from the session
(`gpui::guess_compositor()`): Wayland on a Wayland session. At run time the GPU is reached through
Vulkan (`vulkan-loader` and a driver). One command:

```
sudo dnf install libxcb-devel libxkbcommon-devel libxkbcommon-x11-devel fontconfig-devel freetype-devel wayland-devel vulkan-loader
```

User guide: [`docs/app.md`](../../docs/app.md). Run with a recording: `neuro-convert-app <path>`;
`NC_APP_LOG=1` prints status messages to stderr.

## Layout (MVVM)
```
src/main.rs        application, window, settings, start-up path, NC_APP_LOG
src/actions.rs     actions + key bindings (Ctrl+O / Ctrl+Shift+O open, Ctrl+L / Ctrl+S metadata,
                   Ctrl+Enter convert, Esc cancel, Ctrl+Q quit)
src/state.rs       AppState: phases Empty → Opening → Ready ⇄ Writing; what is enabled
src/settings.rs    preferences, recent files, DANDI switch, remembered values (JSON)
src/domain/        no GPUI, tested directly
  workspace.rs     Workspace: Job, metadata draft, plan, options, progress, report; every change
                   returns the typed Events it caused (begin_* / finish_* around long work)
  events.rs        AppEvent + Events (deduplicated)
  steps.rs         Step (Source / Contents / Metadata / Review), which step fixes an issue, counts
  format.rs        paths, SI units, ticks, ISO ages, time zones, suggestion lists
src/services/      long work on named threads, results over async channels the UI awaits
  mod.rs           run(): one-shot work (open)            sampler.rs  preview, newest request wins
  writer.rs        write + verify, events coalesced
src/store.rs       GPUI entity around the Workspace; emits AppEvents; starts the services
src/viewmodels/    one per screen; subscribes to the events it shows; pure display functions
  nav.rs           current step, enabled steps, issue counts, "reveal" an issue's target
  source.rs        reader cards, recent recordings, detected recording
  contents.rs      tree, issue flags, stream card (signal kind, electrode group, details)
  metadata.rs      session / subject: date-time picker, zone list, age, sex, suggestions
  plan.rs          NWB structure rows and summary       convert.rs  output, options, progress
  preview.rs       stream / channel sets / time window / gain / markers; sampler requests
src/widgets/       RenderOnce pieces: Card, FormRow (issue outline), Choice, MenuSelect,
                   SuggestInput, IssueList (fix links), IncludeToggle, ProgressCard, Traces, ProbeMap
src/views/         render only: source, contents (+ preview), metadata, review
src/app.rs         root: title bar, toolbar, step bar, current step, footer, status bar; UI tests
```

## Tests
```
cargo test -p nc-app
```
Unit tests (domain with a fake reader, services, view-model functions, widgets' geometry) and
headless UI tests in `app.rs` (`#[gpui_kit::test]`, dev feature `gpui-kit/test-support`): steps,
tree checkbox, issue link → field, typing into the form, stream card (kind, new group, location),
container choice, background convert, theme memory, preview following the selection. Real hit
testing, no pixels: look at the window for visuals.
