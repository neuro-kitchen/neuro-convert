//! App actions and their key bindings (`secondary` = Cmd on macOS, Ctrl elsewhere).

use gpui_kit::{actions, App, KeyBinding};

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
    cx.bind_keys([
        KeyBinding::new("secondary-o", OpenRecording, None),
        KeyBinding::new("secondary-shift-o", OpenRecordingFile, None),
        KeyBinding::new("secondary-l", LoadMetadata, None),
        KeyBinding::new("secondary-s", SaveMetadata, None),
        KeyBinding::new("secondary-enter", Convert, None),
        KeyBinding::new("escape", Cancel, None),
        KeyBinding::new("secondary-q", Quit, None),
    ]);
}
