//! App actions and their key bindings (`secondary` = Cmd on macOS, Ctrl elsewhere).

use gpui_kit::{actions, Action, App, KeyBinding};

actions!(
    nc_app,
    [
        /// Choose a recording folder (TDT block / tank, SpikeGLX run) and read it.
        OpenRecording,
        /// Choose a recording file (`.tsq`, SpikeGLX `.bin` / `.meta`) and read it.
        OpenRecordingFile,
        /// Load a metadata YAML.
        LoadMetadata,
        /// Save the metadata as YAML.
        SaveMetadata,
        /// Choose where the NWB store is written.
        ChooseOutput,
        /// Write the planned NWB store.
        Convert,
        /// Stop a running conversion.
        Cancel,
        /// Switch light / dark (stops following the system).
        ToggleTheme,
        Quit,
    ]
);

pub fn bind_keys(cx: &mut App) {
    // `KeyBinding::new` no longer resolves `secondary-`; `load` resolves it for the host OS
    let bindings: [(&str, Box<dyn Action>); 7] = [
        ("secondary-o", Box::new(OpenRecording)),
        ("secondary-shift-o", Box::new(OpenRecordingFile)),
        ("secondary-l", Box::new(LoadMetadata)),
        ("secondary-s", Box::new(SaveMetadata)),
        ("secondary-enter", Box::new(Convert)),
        ("escape", Box::new(Cancel)),
        ("secondary-q", Box::new(Quit)),
    ];
    let bindings: Vec<KeyBinding> =
        bindings.into_iter().map(|(keys, action)| KeyBinding::load(keys, action, None, false, None, cx).expect("valid keystroke")).collect();
    cx.bind_keys(bindings);
}
