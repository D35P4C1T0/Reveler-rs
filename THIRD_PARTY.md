# Third-party software

Reveler-rs is MIT-licensed. Its release archives include available license texts and a Cargo dependency inventory under `licenses/`.

The Rust plugin uses NICE-plug and its core, egui integration and derive crates (ISC), egui (MIT OR Apache-2.0), wgpu (MIT OR Apache-2.0), and their dependencies. See the release's `licenses/rust-dependencies.json` for pinned versions and license declarations.

macOS Audio Units additionally include:

- [clap-wrapper](https://github.com/free-audio/clap-wrapper), v0.16.0, commit `1cca996e96f29ab2be7ae9f8cfe532bbc92e1dd6` (MIT).
- [CLAP SDK](https://github.com/free-audio/clap), 1.2.6 (MIT).
- [Apple AudioUnitSDK](https://github.com/apple/AudioUnitSDK), 1.1.0 (Apache-2.0).
- fmt, supplied by clap-wrapper (MIT).
