# sakiot-dsp

The clip editor's audio processor, compiled natively into `web-server` to
render exports and to WebAssembly for the browser preview, so both run the
same code. The core has no browser or server dependencies; the `wasm` feature
only adds the `wasm-bindgen` boundary. Workspace builds, tests and Clippy cover
the native side, and a dedicated CI job lints the `wasm` feature, builds the
WASM and checks it against native.

## Effects

One fixed chain, whatever order the user adjusts the controls in:

1. reverse;
2. independent pitch and rate (phase vocoder and sinc resampler);
3. the configured silent effect tail (0–30 s), added after rate so it is not
   stretched;
4. volume;
5. bass, mid and treble EQ;
6. waveshaper distortion;
7. feedback delay;
8. stereo chorus;
9. look-ahead compressor;
10. seeded stereo reverb (direct-convolution head, partitioned-FFT tail).

Volume, EQ, distortion, delay and the compressor match Tone.js and Chromium's
Web Audio behavior. The reverb uses a seeded impulse response
instead of Tone's random one, so the same seed and parameters sound the same
everywhere. The compressor's signal model is derived, with attribution, from
Chromium's
[BSD-licensed implementation](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/third_party/blink/renderer/platform/audio/dynamics_compressor.cc).

Saved compositions store parameter values, with advanced effects under
`effects.advanced`. Every parameter is wired into the inspector, the browser
renderer, the saved JSON and the server renderer. Rearrangeable effects would
need a new, explicitly ordered chain model on both sides.

## In the browser

The preview splits the chain where random access ends. A Web Worker renders
and caches reverse, pitch, rate and tail, keyed only by those parameters. The
AudioWorklet (`sakiot-stage/src/features/clip-editor/sharedDspAudioWorklet.ts`)
runs the rest in 128-frame blocks, so downstream controls change playback
immediately without re-rendering. No DSP runs on the main thread.

- The WASM boundary takes one complete, versioned effect object. Unknown
  versions and incomplete objects are rejected.
- Parameter changes ramp over 5 ms in the shared core, so native and WASM
  de-click identically.
- The worker also renders each segment's 2,500-point timeline waveform from the
  exact processed PCM, 120 ms after the last edit, keeping the previous one
  until the new one is ready.
- Clips decode at 48 kHz, the server's rate. If the worker or worklet fails to
  start, a native Web Audio fallback covers volume, EQ, rate and reverse, but
  not independent pitch.
- `Ctrl+Shift+O` (`Command+Shift+O`) opens a hidden JSON editor prefilled from
  the first selected segment. It accepts a complete or partial camelCase
  `SegmentEffects` object, applies it to every selected segment, rejects
  unknown keys, wrong types and out-of-range values, and tolerates Markdown
  `json` fences.

### Browser limits

Preprocessing uses `WasmIncrementalRenderer` in 4096-frame blocks. Decoded
sources share a 64 MiB LRU cache (fetched and decoded one at a time), and
processed PCM, waveforms and Web Audio copies share another 64 MiB. Those
budgets cover retained payloads, not the whole heap: `decodeAudioData` still
decodes whole clips, and buffers held by the editor or playback sit outside
them. Preview is disabled with a notice when an unmuted segment's source window
or output exceeds 64 MiB of stereo PCM (about 175 s at 48 kHz), or total
unmuted output exceeds 128 MiB; keyboard playback follows the same limit and
export stays available. Removing that limit would take a windowed or seekable
decoder and streamed playback.

## On the server

Exports use renderer version 2: FFmpeg decodes each source window to stereo
48 kHz `f32` PCM, `IncrementalRenderer` processes it, and FFmpeg places, mixes
and Opus-encodes the result. Placement concatenates generated silence rather
than using `adelay`.

`IncrementalRenderer::new(rate, channels, input_frames, effects)` declares the
exact decoded length. `push(block, sink)` accepts aligned blocks of any size
and emits at most 512 frames per sink call; `finish(sink)` drains the overlap
and appends the tail once. Incomplete or excess input, misaligned samples and
a second `finish` are errors, and a sink failure makes the renderer terminal.
State (vocoder history and phases, resampler position, every effect) persists
across calls, and memory depends on the FFT size (2048), hop (512), resampler
radius (24) and the configured delay and reverb times, not on segment length.

The renderer rejects `reverse: true`. The server instead reads its decoded PCM
file backwards in 4096-frame blocks, reverses whole stereo frames, and renders
forwards. Decoded and processed PCM live as files in the job's attempt
directory, one segment at a time; processed segments stay until the final mix.
The job limits in [web-server](../web-server/README.md#composition-exports)
still bound time and disk, so very large or overlapping compositions can fail
explicitly.

The whole-buffer renderer remains the numerical reference. Both use `f64`
source positions and the same pinned `libm` math, so long inputs and extreme
stretches agree across platforms.

## Verification

`tests/incremental.rs` compares the incremental renderer with the reference
across block sizes (down to one frame), short and empty input, fractional
rate and pitch, the full stateful chain, reverse, tails and a 61-second stereo
source. The tolerance is a maximum absolute error of 2e-6 and an RMS error of
2e-7, and the same bound applies native against WASM (`web/verify-wasm.mjs`,
which CI runs), including +4800 cents at 0.1x and -4800 cents at 10x. The
compiled WASM is not byte-compared across hosts, because rustc can encode it
differently with identical output.

Release-build measurements on the development host (2026-09-06), stereo
48 kHz, with distortion, delay, compressor, chorus, a 1.5 s seeded reverb and a
0.5 s tail:

| Source | Pitch / rate | Peak RSS | Wall time |
| --- | --- | --- | --- |
| 60 s | +700 cents / 1.35 | 8,140 KiB | 9.97 s |
| 600 s | +700 cents / 1.35 | 8,016 KiB | 112.76 s |
| 6 s | +4800 cents / 0.1 | 8,028 KiB | 31.02 s |
| 60 s, maximum delay, chorus, reverb and tail | none | 123,384 KiB | 35.05 s |

These cover the DSP alone, consuming output as it is produced. Memory stays
flat with duration; CPU time and disk do not. Reproduce from the repository
root:

```sh
cargo test -p sakiot-dsp --locked --release
cargo build -p sakiot-dsp --locked --release --example measure_incremental
/usr/bin/time -v target/release/examples/measure_incremental 60 700 1.35
/usr/bin/time -v target/release/examples/measure_incremental 600 700 1.35
/usr/bin/time -v target/release/examples/measure_incremental 6 4800 0.1
/usr/bin/time -v target/release/examples/measure_incremental 60 0 1 max-effects
```

`process_raw` and `render_clip_raw` run the streaming chain and the
length-changing renderer over interleaved little-endian `f32` PCM:

```sh
cargo run -p sakiot-dsp --example process_raw -- \
  48000 2 -3 5 -4 2.5 0.7 1 0.125 0.4 1 true -24 30 12 0.003 0.25 \
  true 1.5 3.5 0.7 180 0 0.5 true 1.5 0.01 0.5 1396788041 \
  <input.f32 >output.f32
cargo run -p sakiot-dsp --example render_clip_raw -- \
  48000 2 700 1.35 true 2.0 <input.f32 >output.f32
```

### Real-browser harness

`bun run browser:measure` (after `bun install` and
`bunx playwright install chromium` in this directory) renders pinned Tone.js
through a 48 kHz `OfflineAudioContext` and compares it with direct WASM and
with a real-time Chromium AudioWorklet capture. Tone is only a comparison
oracle here; the frontend does not ship it. Worklet captures of each effect
and of the full chain are bit-identical to direct WASM when both reset at the
same frame, which production guarantees by passing each worklet node an
explicit start frame.

## Building the WASM

`pkg/` is build output. `scripts/build-dsp.sh` is the only place that names the
build steps; `bun run wasm:build` here, `bun run build:dsp` in `sakiot-stage`
and CI all call it. It enforces the pinned `wasm-bindgen` and `rolldown`
versions. Each machine that builds the frontend needs, once:

```sh
cargo install wasm-bindgen-cli --version "$(cat sakiot-dsp/wasm-bindgen-cli-version)" --locked
(cd sakiot-stage && bun install)
scripts/build-dsp.sh
node sakiot-dsp/web/verify-wasm.mjs   # optional native/WASM parity check
```

`rust-toolchain.toml` already installs the `wasm32-unknown-unknown` target.
The frontend consumes `pkg/sakiot_dsp.js`, the WASM file and the bundled
`pkg/sakiot-dsp-worklet.bundle.js`.

## Known gaps

- Seeking into the middle of a segment starts delay, compressor, chorus and
  reverb fresh instead of with the preceding history. Playback from a segment
  start matches the server exactly.
- A WASM load failure shows no error or retry.
- The worker cannot interrupt a render in progress; it drops obsolete queued
  requests instead.
- The phase vocoder, resampler and reverb convolution are unprofiled, and any
  optimization must keep native and WASM identical.
