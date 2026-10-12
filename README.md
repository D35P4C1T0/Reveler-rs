# Reveler-rs

[![Build](https://github.com/D35P4C1T0/Reveler-rs/actions/workflows/build.yml/badge.svg)](https://github.com/D35P4C1T0/Reveler-rs/actions/workflows/build.yml)

A Rust audio leveler with adaptive envelopes, frequency-weighted loudness detection, lookahead true-peak limiting, and an egui/wgpu editor.

## Download

Get your platform ZIP from [Releases](https://github.com/D35P4C1T0/Reveler-rs/releases/latest).

| Platform | Architecture | Formats |
|---|---|---|
| Windows | x86-64 | VST3, CLAP |
| Linux | x86-64, Ubuntu 24.04+ | VST3, CLAP |
| macOS | Intel + Apple Silicon, 10.15+ | VST3, CLAP, AUv2 |

VST3 and CLAP support mono, stereo, and 5.1 with independent LFE processing. AUv2 supports stereo and includes its CLAP engine. Mac binaries are ad-hoc signed, without Apple notarization.

Extract the archive, copy the complete plugin bundle to your DAW's plugin directory, and rescan.

## Controls

Target Level, True Peak, Max Gain, Freeze Level, Input Level, separate High/Low Correction, four linear/logarithmic Mix Modes, LFE Gain, and latency-compensated Bypass. Latency is 1698 samples at 48 kHz. Processing, parameter updates, and reset allocate no memory.

## Build

Requires Rust 1.95+; CI uses 1.99. The standalone DSP requires 1.88.

```sh
cargo test --workspace --release --locked
cargo xtask bundle reveler-rs --release --locked
```

[GitHub Actions](.github/workflows/build.yml) builds all three platforms, including universal Mac bundles and AUv2. Tagged builds publish ZIP artifacts and SHA-256 checksums to GitHub Releases after all checks pass.

MIT licensed. See [third-party notices](THIRD_PARTY.md).
