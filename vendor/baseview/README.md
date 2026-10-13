# baseview 0.3.7

Vendored from RustAudio/baseview commit
`5f2c138794d5f0a46582fce186fb7fbd8280eea7` (crates.io 0.3.7).
Licensed under MIT OR Apache-2.0; both license texts are included.

Local macOS patch: install Rust view state after `NSView.initWithFrame:`
returns. AppKit can re-enter subclass callbacks during initialization;
accessing tracking areas before NSView finishes initialization aborts the host.
The optional `metal-layer` feature provides a native CAMetalLayer backing layer,
avoiding process-global raw-window-metal helper-class collisions when multiple
plugin binaries share a host. Reveler-rs enables it only on macOS.
Windows and Linux source is unchanged.

`tools/mac_gui_smoke.py` exercises the early callback and editor lifecycle.
