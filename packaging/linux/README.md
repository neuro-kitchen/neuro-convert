# Linux packaging

Status: the desktop entry is ready (validated with `desktop-file-validate`); no package is built
yet. Building and testing a Flatpak or AppImage is open work.

## Install for the current user (no package)
```
cargo build --release -p nc-app
install -Dm755 target/release/neuro-convert-app ~/.local/bin/neuro-convert-app
install -Dm644 packaging/linux/org.neuro-kitchen.neuro-convert.desktop \
    ~/.local/share/applications/org.neuro-kitchen.neuro-convert.desktop
```
The entry's `StartupWMClass` matches the window's `app_id` (`org.neuro-kitchen.neuro-convert`),
so the desktop groups the window with its launcher. `Exec=neuro-convert-app %f` opens a recording
passed by "Open with".

## Run-time requirements
Wayland or X11, a Vulkan driver (`vulkan-loader`), fontconfig / freetype, and the xdg desktop
portal (file dialogs go through `org.freedesktop.portal.FileChooser`). Build requirements:
`crates/app/README.md`.

## Not done
- Flatpak manifest (GNOME runtime + Rust SDK extension; portal permissions are the default way the
  app reaches files, so no broad filesystem access is needed for dialogs; drag and drop and
  "open with" paths need `--filesystem=home` or the document portal).
- AppImage (needs bundling of the Vulkan loader decision: use the host's).
- An app icon (none designed yet).
