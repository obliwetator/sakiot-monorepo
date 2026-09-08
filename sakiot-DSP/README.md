# sakiot-DSP

The shared Rust audio processor for the Sakiot clip editor, usable as native
server code and WebAssembly in the browser.

The crate remains excluded as a root-workspace member. The web server consumes
it as a path dependency, so normal server builds compile the native crate; a
dedicated CI job additionally checks standalone tests, native/WASM parity, and
that the committed WASM bindings are current. See [PLAN.md](PLAN.md) for scope,
status, and recorded findings.

Implemented in the shared native/WASM core:

- volume and three-band Tone-compatible EQ;
- Tone-compatible waveshaper distortion;
- delay and feedback delay, including Web Audio feedback-cycle timing;
- Chromium/Tone-compatible look-ahead compression;
- default sine-wave stereo chorus;
- deterministic seeded stereo reverb with a direct-convolution head and
  partitioned-FFT tail;
- an offline clip renderer for independent rate and pitch, plus frame-safe
  reverse before the streaming effects;
- an explicit 0-30 second per-segment effect tail, appended after
  reverse/pitch/rate and processed through the streaming effects.

Tone's random reverb IR is intentionally replaced by a seeded IR: the same
seed and parameters now identify the same sound in native and WASM builds.
Length-changing rate/pitch work is exposed through the offline clip renderer
rather than forced into the in-place AudioWorklet effect loop.

## Tone.js coverage

The shared core and product integration implement volume, three-band EQ,
independent pitch and rate, reverse, distortion, feedback delay, compressor,
chorus, and deterministic reverb.

This is not the complete Tone.js 15.1.22 effect catalog. Still absent are
AutoFilter, AutoPanner, AutoWah, BitCrusher, Chebyshev, FrequencyShifter,
Freeverb, JCReverb, Phaser, PingPongDelay, StereoWidener, Tremolo, and Vibrato.
The shared reverb and pitch shifter intentionally replace Tone's algorithms
rather than reproduce them independently: native and WASM use one canonical
implementation.

Every implemented effect parameter is wired into the editor inspector, browser
WASM renderer, saved composition JSON, and native server renderer.

## Effect processing order

The DSP owns one fixed signal chain:

1. reverse;
2. independent pitch/rate transform;
3. append the configured silent effect tail;
4. segment volume;
5. bass, mid, then treble EQ;
6. distortion;
7. feedback delay;
8. chorus;
9. compressor;
10. reverb.

The order in which a user adjusts inspector controls does not change this
chain. Saved compositions store the current parameter values, not the editing
chronology. Supporting rearrangeable effects would require a new, explicitly
ordered effect-chain model, with compatible serialization and execution in
both the browser WASM and native server renderers.

## Product integration

- Browser previews split the fixed chain at its natural random-access boundary.
  A dedicated Web Worker renders and caches reverse/pitch/rate/tail using a key
  containing only those geometry parameters. The production AudioWorklet then
  runs volume, EQ, distortion, delay, chorus, compressor, and reverb in 128-frame
  blocks through the same Rust processor. Those downstream controls update
  active playback immediately and no longer restart or await a complete segment
  render. The worker independently runs the streaming suffix against a copy of
  the cached geometry for the exact processed waveform. Geometry jobs have
  priority, and obsolete queued waveform jobs are replaced by the newest edit.
  No DSP executes synchronously on the main thread. Clip decoding is fixed at
  the server's canonical 48 kHz rate; a limited native Web Audio fallback
  remains for worker/worklet initialization failure.
- The WASM boundary accepts one complete versioned effect object instead of a
  positional parameter list. Schema version 2 includes `tailSeconds`; unknown
  versions and incomplete configurations are rejected.
- Live parameter changes use a deterministic 5 ms output-continuity ramp in
  the shared core, so native and WASM updates de-click over the same number of
  samples without interpolating unstable nonlinear/stateful coefficients.
- Timeline segment waveforms are generated inside the DSP worker from exact
  complete-chain PCM. The 2,500-point envelope
  updates after the same 120 ms edit debounce and falls back to the source
  waveform until the first render is available. Later edits retain the
  preceding processed envelope until its replacement is ready; reverse changes
  mirror it optimistically, preventing a flash of the raw-source waveform.
- The Effect tail inspector control extends the timeline box by a fixed number
  of seconds. That appended silence is part of the exact processed waveform,
  cached preprocessing AudioBuffer, saved composition, and server output. Rate changes only
  scale source content, not the tail. Splitting keeps the configured tail on
  the final timeline piece so it is not duplicated.
- A hidden effect-settings JSON editor opens with `Ctrl+Shift+O` (or
  `Command+Shift+O`). It is prefilled from the first selected segment and
  accepts a complete or partial camelCase `SegmentEffects` object, applying
  supplied values to every selected segment. It rejects unknown keys, wrong
  types, and out-of-range values; Markdown `json` fences are accepted for
  convenient copy/paste from development conversations.
- Server compositions decode each source window to stereo 48 kHz `f32`, call
  the same Rust renderer, and leave placement, mixing, and Opus encoding to
  FFmpeg.
- Advanced effects are grouped under `effects.advanced` in composition JSON.
  Every server segment uses the incremental shared DSP renderer, including
  source windows longer than 60 seconds. Decoded and processed PCM use
  temporary files; processing retains bounded blocks.
- Tone.js has been removed from `sakiot-stage` and its dependency lockfile.
- Browser preview has explicit PCM size limits; oversized compositions remain
  exportable by the server. See [STREAMING.md](STREAMING.md) for these limits
  and the server job time/disk budgets.

## Native checks

```sh
cargo test --manifest-path sakiot-DSP/Cargo.toml
cargo run --manifest-path sakiot-DSP/Cargo.toml --example process_raw -- \
  48000 2 -3 5 -4 2.5 0.7 1 0.125 0.4 1 true -24 30 12 0.003 0.25 \
  true 1.5 3.5 0.7 180 0 0.5 true 1.5 0.01 0.5 1396788041 \
  <input.f32 >output.f32
```

The raw utility reads and writes interleaved little-endian `f32` PCM.

The length-changing offline boundary has a smaller diagnostic utility:

```sh
cargo run --manifest-path sakiot-DSP/Cargo.toml --example render_clip_raw -- \
  48000 2 700 1.35 true 2.0 <input.f32 >output.f32
```

## WASM assets

`pkg/` is committed so the frontend, CI, and the deploy engine can build
without a WASM toolchain. Regenerate it with the repository script after any
change to the DSP sources or to the pinned tooling; it enforces the pinned
`wasm-bindgen` and `rolldown` versions and always builds from `sakiot-DSP`, so
the output does not depend on the caller's working directory. From the
repository root:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version "$(cat sakiot-DSP/wasm-bindgen-cli-version)" --locked
(cd sakiot-DSP && npm ci)
scripts/build-dsp.sh
(cd sakiot-DSP && node web/verify-wasm.mjs)
```

`sakiot-stage` consumes the generated `pkg/sakiot_dsp.js`, WASM asset, and
bundled `pkg/sakiot-dsp-worklet.bundle.js`. The worker owns random-access
preprocessing and exact waveform rendering; the AudioWorklet is the production
real-time boundary for the downstream chain.

The script is the only place that names the build steps: `npm run wasm:build`
delegates to it, and the CI `dsp` job calls it directly.

To verify the generated WASM processor against the native processor with the
version-matched `wasm-bindgen` CLI and Node.js (from `sakiot-DSP/`):

```sh
scripts/build-dsp.sh
(cd sakiot-DSP && node web/verify-wasm.mjs)
```

CI runs the parity verifier once against the committed browser asset and again
after rebuilding it. The generated JavaScript, TypeScript declarations, and
worklet bundle must also remain byte-for-byte clean. The compiled WASM module
is not byte-compared across build hosts because rustc can produce different
binary encodings with sample-identical behavior; its native parity is the
release gate.

## Real-browser parity harness

The standalone harness serves the installed Tone.js 15.1.22 modules, renders
Tone through a 48 kHz `OfflineAudioContext`, and compares it with both direct
WASM and a real-time Chromium `AudioWorklet` capture.
Tone is retained here only as a development comparison oracle; the production
`sakiot-stage` package no longer depends on or bundles it. The harness pins and
serves its own Tone dependency, so it does not rely on frontend packages.

```sh
cd sakiot-DSP
npm install
npx playwright install chromium
npm run browser:measure
```

On Linux, Chromium's normal system libraries must also be present. This
environment lacked `libnspr4` and `libnss3`, so they were temporarily extracted
and supplied through `SAKIOT_BROWSER_LIBRARY_PATH` rather than installed
system-wide.

Initial Chromium 151 results for the combined volume/EQ chain:

- Tone versus WASM mixed fixture: approximately -96 dB relative residual.
- Tone versus WASM sweep fixture: approximately -98 dB relative residual.
- Captured AudioWorklet versus direct WASM: bit-identical for 8,192 frames.

Later slices measured standalone distortion around -124 dB relative residual,
integer delay/feedback below -300 dB, and compression around -82 to -84 dB.
The compressor signal model is derived with attribution from Chromium's
[BSD-licensed implementation](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/third_party/blink/renderer/platform/audio/dynamics_compressor.cc).

The real-time worklet harness also runs basic EQ, distortion, feedback delay,
compressor, chorus, reverb, and the complete active chain separately. All seven captured
cases are bit-identical to direct WASM once both processors are reset at the
same segment-time origin. The harness detects the first fixture signal only as
a test convenience. Production passes an explicit absolute start frame to each
worklet node and resets on that frame, including starts inside a render quantum.

The Node verification also runs reverse with +700 cents at 1.35x through the
offline WASM and native APIs. The initial prototype measured about -128 dB
relative residual with identical output length; renderer version 2 measurements
and native/WASM tolerances are recorded in [STREAMING.md](STREAMING.md).

See [incremental composition rendering](STREAMING.md) for the block API, server resource measurements, renderer-version migration and remaining browser memory limits.
