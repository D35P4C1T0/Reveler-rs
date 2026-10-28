# Local changes to nice-plug 0.4.2

Source: crates.io nice-plug 0.4.2, upstream commit
`91ba75d20c43ca0cd21afa802d0ae8c0511f48ed`.
The normalized published manifest is retained. The original workspace-dependent
manifest is omitted; Cargo.lock is regenerated for standalone tests. LICENSE is supplied from
upstream's main branch (ISC, Robbert van der Helm 2022–2024).

## VST3 editor callback thread checks

Carla on Linux can dispatch `IEventHandler::onFDIsSet` on its engine thread,
which is different from the thread that created the editor. Previously,
`Task::CallMainThread` accessed `Fragile<SpawnedEditor>` with `get()`, panicked,
and then aborted because the panic crossed a C ABI callback.

`src/wrapper/vst3/util.rs::with_editor_window` checks for a conflicting borrow
and uses `Fragile::try_get()` before invoking editor code. The state-change,
parameter-value, and main-thread notification tasks use this helper. Invalid
callbacks are logged and skipped. Editor ownership checks remain intact; no
editor objects are made Send/Sync and no callbacks are moved to a worker thread.

Regression tests deliberately dispatch a callback from a second thread against
an editor containing `Rc<Cell<_>>`. They check that this neither invokes editor
code nor panics, and that subsequent owner-thread callbacks still work. Closed
editors and callbacks concurrent with editor mutation are also covered.

This is crash prevention, not a main-thread dispatcher. A skipped notification
is not retried; host resize negotiation or notifications can be incomplete in a
host that delivers callbacks on the wrong thread. Fixing that requires a real
owner-thread dispatch mechanism in the host or framework. The Linux fallback
executor is a background worker and cannot provide that guarantee.

Run the focused tests from the repository root:

```sh
cargo test --manifest-path vendor/nice-plug/Cargo.toml --lib editor_thread_tests --release --locked --target-dir target
```

## VST3 speaker layout validation

`setBusArrangements` now checks the speaker maps returned by
`getBusArrangement`, rather than accepting any layout with the same channel
count. In particular, six-speaker layouts without LFE are rejected instead
of being processed as 5.1. Auxiliary bus indices also follow the main bus.
The plugin's `vst3_accepts_only_the_speaker_maps_it_reports` test checks
mono, stereo, and 5.1, and rejects unsupported input and output maps.

## Carla validation

`tools/carla_gui_smoke.py` opens an independent host using the built VST3,
resizes its X11 child, changes a host parameter, and closes/reopens the editor
three times. Run it with and without `--unbridged` in a desktop session with
Carla, PyQt6 and JACK available. Set `NICE_LOG` to capture rejected callbacks.

On Linux with Carla 2.6.0-alpha1, both modes pass against the patched release
bundle, including clean shutdown. Against the original v0.2.0 bundle, the same
test reproduces the `inner.rs:754` ownership panic and fails after the bridge
aborts. Host parameter values are checked; this test does not verify visual
parameter feedback or successful parent-window resize negotiation.

## REAPER compatibility validation

On Linux with REAPER 7.82, the patched VST3 and the installed v0.2.0 VST3
both pass an identical isolated-profile comparison: three editor open/close
and direct X11 resize cycles, host parameter updates, bypass, nonzero audio
processing, saved-state restoration into a fresh instance, project save/reload,
a second independent instance, plugin removal, and normal host shutdown.
The test asserts the loaded bundle's full path to distinguish the builds.
The patched run logs no warnings, panics, or rejected editor callbacks.
Screenshots confirm live meters and host-updated parameter text.
A separate playing automation-envelope test also passes for both builds with
the editor open and audio processing active. In the patched run, the automated
normalized parameter advances from approximately 0.31 to 0.72 without any
rejected callbacks, and REAPER exits normally afterward.

The synthetic mouse check for the More Controls expand/collapse button did
not obtain the expected window-height change in either build. That control
is not counted as verified; these results do not establish the cause of the
mouse-test failure. The comparison uses REAPER's dummy audio device with a
generated stereo tone, not a physical audio interface or an offline render.
Other DAWs and Windows/macOS REAPER remain untested.
