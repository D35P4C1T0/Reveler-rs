# Third-party software

Reveler-rs is MIT-licensed. Its release archives include available license texts and a Cargo dependency inventory under `licenses/`.

The Rust plugin uses NICE-plug and its core, egui integration and derive crates (ISC), egui (MIT OR Apache-2.0), wgpu (MIT OR Apache-2.0), and their dependencies. See the release's `licenses/rust-dependencies.json` for pinned versions and license declarations.

[baseview](https://github.com/RustAudio/baseview) 0.3.7 (MIT OR Apache-2.0) is vendored with a macOS view-initialization fix. Its source and licenses are in `vendor/baseview`.

[nice-plug](https://codeberg.org/RustAudio/nice-plug) 0.4.2 (ISC) is vendored with checks that reject VST3 editor notifications delivered on a different thread from the editor's creation thread. This prevents a panic and process abort observed in Carla on Linux. VST3 speaker maps are also validated against the supported layouts. Its source, license, and patch notes are in `vendor/nice-plug`.

macOS Audio Units additionally include:

- [clap-wrapper](https://github.com/free-audio/clap-wrapper), v0.16.0, commit `1cca996e96f29ab2be7ae9f8cfe532bbc92e1dd6` (MIT), with an AU Cocoa view initialization fix in `packaging/au/patch_cocoa_view.py`.
- [CLAP SDK](https://github.com/free-audio/clap), 1.2.6 (MIT).
- [Apple AudioUnitSDK](https://github.com/apple/AudioUnitSDK), 1.1.0 (Apache-2.0).
- fmt, supplied by clap-wrapper (MIT).
