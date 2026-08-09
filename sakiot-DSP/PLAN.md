# Sakiot DSP prototype plan

Last updated: 2026-08-09

## Goal

Prove that one deterministic Rust DSP core can process clip-editor audio in
both native code and WebAssembly closely enough that a browser preview and a
server render are perceptually indistinguishable.

This directory started independent from the root Cargo workspace and frontend.
It remains excluded as a root-workspace member. The validated core is consumed
by the frontend and by the server as a path dependency, and now has a dedicated
CI suite for standalone native/WASM validation and generated-artifact checks.

## Current editor contract to mirror

The existing clip editor applies these operations per segment:

1. Trim and optional reverse.
2. Playback rate and duration-preserving pitch shift.
3. Append the explicitly configured silent effect tail.
4. Segment volume in dB.
5. 250 Hz low-shelf bass gain.
6. 1 kHz peaking mid gain with Q = 1.
7. 3 kHz high-shelf treble gain.
8. Timeline placement, summing, and master volume.

The browser and server now use the shared offline clip renderer for every
implemented operation: reverse, independent rate, duration-preserving pitch,
segment volume, three-band EQ, distortion, feedback delay, compression,
chorus, and deterministic reverb. Web Audio schedules the pre-rendered browser
buffers. FFmpeg decodes, places, mixes, and encodes native server renders.

Effect processing order is owned by the DSP and is fixed: reverse, pitch/rate,
append the configured silent tail, volume, bass/mid/treble EQ, distortion,
feedback delay, chorus, compressor, then reverb. Adjusting controls in a
different order does not alter the signal chain. Compositions persist parameter
values rather than editing chronology.
Rearrangeable processing would require an explicit ordered effect-chain model
and shared browser/server serialization and execution semantics.

## Work plan

- [x] Inspect and record the current Tone/FFmpeg effect order and parameters.
- [x] Create an isolated Rust crate outside the root workspace member list.
- [x] Implement a block-size-independent mono/stereo `f32` processing core.
- [x] Implement volume and matching low-shelf, peaking, and high-shelf biquads.
- [x] Expose the same processor through a small native API and an optional
      `wasm-bindgen` API suitable for an `AudioWorklet` spike.
- [x] Add deterministic unit tests for bypass, gain, channel isolation,
      parameter changes, reset, and block-size independence.
- [x] Add a native raw-PCM utility and an FFmpeg comparison test.
- [x] Build the native target and the
      `wasm32-unknown-unknown` target.
- [x] Generate browser bindings and execute the WASM processor against native.
- [x] Record initial parity measurements and integration findings here.

## Next prototype slice

- [x] Render the current Tone.js graph in a real 48 kHz `OfflineAudioContext`
      and compare it with shared WASM using impulse, sweep, and mixed fixtures.
- [x] Capture the shared processor through a real 48 kHz Chromium
      `AudioWorklet` and compare that output with direct WASM.
- [x] Add a standalone browser harness under this directory; do not modify the
      production Vite application yet.
- [x] Choose the native server boundary: decode source windows with FFmpeg,
      process native stereo `f32` through the Rust crate, then return placement,
      mixing, and encoding to FFmpeg.
- [x] Add click-free parameter smoothing with identical sample-based behavior
      in WASM and native builds. Live updates preserve the preceding output
      sample and decay a correction over exactly 5 ms, avoiding unstable
      interpolation inside nonlinear and stateful processors.
- [x] Decide the canonical implementation for rate and pitch. Do not reproduce
      Tone PitchShift and Rubber Band as two independent algorithms.
- [x] Reverse complete interleaved frames at the offline source boundary before
      the shared rate/pitch transform and streaming effects.
- [x] Render a real 48 kHz stereo production clip through standalone and
      combined presets for direct listening from the repository workspace.
- [x] Add Tone-compatible distortion as the first new shared effect after
      browser validation. Mirror Tone's effective 1,024-point
      `WaveShaper` curve and measure the browser interpolation behavior rather
      than assuming the analytic shaping function is sample-identical. Tone
      constructs a 4,096-point shaper but its amount setter silently replaces
      it using `setMap`'s 1,024-point default.
- [x] Add a shared delay/feedback-delay line and validate exact timing,
      feedback, wet/dry mixing, and block-boundary behavior against Tone.
- [x] Prototype a canonical compressor. Treat Tone/Web Audio parity as a
      measurement target, since native `DynamicsCompressorNode` behavior is not
      specified tightly enough to serve as the server implementation.
- [x] Add the default sine-wave Tone chorus signal model using stereo
      modulated delays, spread, feedback, and equal-power wet/dry mixing.
- [x] Evaluate Tone reverb architecture. Exact parity cannot be parameter-only:
      Tone generates a new random stereo noise impulse response. Choose a
      deterministic shared IR and partitioned convolution before implementing
      reverb in the real-time core.
- [x] Implement deterministic seeded impulse-response generation and a
      block-size-independent partitioned convolver for shared reverb.
- [x] Replace the growing positional WASM setter with a versioned effect-config
      boundary before adding another DSP generation. Version 2 accepts one
      complete `{ version, effects }` object and rejects unknown versions or
      missing fields, including the explicit tail duration.
- [x] Define and integrate an explicit fixed effect-tail duration policy. The
      browser and native renderers append 0-30 seconds after reverse/pitch/rate,
      then process it through the shared effect chain. Timeline geometry,
      waveform PCM, playback, saved compositions, validation, and server output
      all use that same extent.
- [x] Add dedicated CI coverage without making the crate a root workspace
      member: formatting, clippy, native tests, pinned WASM/worklet generation,
      native/WASM parity, and a clean generated-artifact check.

## Prototype acceptance criteria

- Native processing is deterministic across repeated runs.
- Processing a signal in 128-frame blocks produces the same samples, within
  floating-point tolerance, as processing it in differently sized blocks.
- Mono and stereo channel state remain independent.
- The WASM wrapper calls the exact same Rust processing implementation.
- The shared EQ response is sufficiently close to the current FFmpeg chain to
  make a listening comparison meaningful; any measurable mismatch is recorded.
- Dedicated CI fails if native tests/parity regress or generated WASM artifacts
  become stale.

## Effect catalog boundary

- Product contract integrated now: volume, three-band EQ, independent pitch
  and rate, reverse, distortion, feedback delay, compressor, chorus, and
  deterministic reverb, including every parameter in `SegmentEffects`.
- Tone.js 15.1.22 effects not implemented: AutoFilter, AutoPanner, AutoWah,
  BitCrusher, Chebyshev, FrequencyShifter, Freeverb, JCReverb, Phaser,
  PingPongDelay, StereoWidener, Tremolo, and Vibrato.
- Shared reverb and pitch are canonical replacements, not independent replicas
  of Tone's random ConvolverNode IR and delay-LFO PitchShift algorithms.

## Integration added now

- Browser: generated WASM bindings, eager initialization, deterministic
  per-source/effect caching, offline segment rendering, and native Web Audio
  scheduling/master gain. Browser clip decoding is fixed at the server's
  canonical 48 kHz rate. Every shared effect parameter is exposed in grouped
  inspector controls and participates in the cache key. Playback waits while
  WASM loads. Tone.js and its frontend transitive dependencies are removed.
  Timeline waveforms now reduce the exact cached, effect-processed PCM into
  2,500 min/max points client-side, with the source waveform as a loading/error
  fallback.
- Server: native `sakiot-dsp` dependency, exact source-window decode to stereo
  48 kHz `f32`, native rendering of the complete effect contract, temporary
  raw file cleanup, and FFmpeg placement/mix/Opus encoding. The API stores the
  advanced effect group, deterministic reverb seed, and explicit tail duration
  with the composition.
- Verification: frontend unit/type/bundle checks, server unit/clippy checks,
  and an ignored real-media test exercising decode -> DSP -> mix -> Opus with
  production clip `8161a145-9b3d-4bdb-bca1-4d12fd6781a2`.
- CI: DSP changes select the Rust, frontend, and dedicated DSP suites. The DSP
  suite pins the wasm-bindgen CLI and rebuilds/checks the committed browser
  package in addition to the native tests inherited through the server.

## Still missing or deliberately deferred

- AutoFilter, AutoPanner, AutoWah, BitCrusher, Chebyshev, FrequencyShifter,
  Freeverb, JCReverb, Phaser, PingPongDelay, StereoWidener, Tremolo, and Vibrato.
- A user-reorderable effect-chain model. The current parameter object
  intentionally represents one fixed DSP-owned order and contains no editing
  chronology.
- Worker/AudioWorklet-based background rendering. Current browser pre-renders
  synchronously after WASM initialization and can block the main thread on
  long clips or frequent uncached parameter changes.
- Streaming/chunked length-changing DSP. Server segments over 60 seconds still
  take the legacy FFmpeg/Rubber Band path to cap temporary memory use.
- A sample-accurate placement path; current FFmpeg `adelay` placement rounds
  to milliseconds.
- A visible browser error/retry state for WASM load failure. The emergency
  native fallback covers volume, EQ, rate, and reverse, but cannot reproduce
  independent pitch.

## Decisions and findings

- The package has its own `[workspace]` and is explicitly excluded from root
  workspace membership. It is a web-server path dependency, so root server
  builds compile it transitively; the dedicated CI job covers the standalone
  tests and WASM artifact boundary that transitive compilation cannot.
- With the current effect set, versioned JavaScript configuration reader, and
  offline renderer, the generated WASM module is 88,253 bytes (Vite reports
  88.25 kB and 37.32 kB gzip) in this environment.
- A 4,097-frame stereo fixture processed through WASM and native Rust was
  bit-identical (`max_abs = 0`, reported relative residual below -6000 dB).
- Volume and the 1 kHz peaking band already track the current FFmpeg graph very
  closely: approximately -99 dB and -98 dB relative residual respectively.
- The current FFmpeg `bass=t=s:w=1` and `treble=t=s:w=1` settings do not use the
  same shelf shape as the canonical RBJ/Web Audio-style coefficients. On the
  mixed fixture they measured approximately -27 dB and -36 dB relative
  residual. The full current chain measured approximately -28 dB.
- Supplying the shared coefficients to FFmpeg's generic `biquad` filter with
  transposed direct form II and `f32` precision reduced the full-chain relative
  residual to approximately -98 dB. This is the leading low-risk server bridge
  for volume/EQ; the browser measurements below independently validate the
  Tone side.
- Chromium 151 measured the current Tone graph against shared WASM at about
  -96 dB relative residual for the combined mixed fixture and about -98 dB for
  the combined sweep fixture. The impulse fixture measured about -111 dB.
- The 8,192-frame real-time AudioWorklet capture was bit-identical to direct
  WASM (`max_abs = 0`). This validates the real browser processing boundary,
  not only JavaScript calls into the generated WASM wrapper.
- Chromium did not register the imported wasm-bindgen module directly. A
  reliable worklet boundary required bundling it into one module, transferring
  a precompiled `WebAssembly.Module`, initializing synchronously in the node
  constructor, and providing a minimal `TextDecoder` fallback for the restricted
  `AudioWorkletGlobalScope`.
- Reverse remains a scheduling/frame-order operation. Pitch and rate remain
  outside the in-place streaming processor; the shared offline clip renderer
  now owns these length-changing transforms.
- The canonical offline rate/pitch path uses a 2,048-frame phase vocoder and a
  48-tap Blackman-windowed sinc resampler. It first stretches by
  `pitch_ratio / rate`, then resamples by `pitch_ratio`, which keeps rate and
  requested pitch independent while producing exactly `input_frames / rate`.
  A 1.5x rate kept a 440 Hz fixture at 440 Hz; +1,200 cents produced 880 Hz at
  unchanged duration. A combined +700-cent/1.35x native-versus-WASM fixture
  measured -117.55 dB relative residual with identical output length. Browser
  integration now pre-renders the clip into an AudioBuffer; an in-place
  AudioWorklet cannot represent a length-changing operation cleanly.
- Tone 15.1.22's `Distortion` constructor requests a 4,096-point WaveShaper,
  but assigning its amount calls `setMap` without that length and replaces the
  curve with the 1,024-point default. The shared implementation mirrors the
  effective 1,024-point curve and browser interpolation.
- Tone distortion alone measured bit-identical on the impulse fixture and
  about -124 dB relative residual on sweep and mixed fixtures. The combined
  volume/EQ/distortion chain measured about -95 dB on mixed audio and -99 dB
  on the sweep. Disabled Tone effects must be removed from the graph: leaving
  a `wet: 0` effect node connected introduced roughly -50 dB residual through
  its crossfade path.
- Tone/Web Audio feedback cycles add one 128-frame render quantum on every
  feedback traversal. A 6,000-frame delay therefore echoes at frame 6,000,
  then 12,128, 18,256, and so on. The compatibility implementation models the
  direct delay and this feedback-cycle quantum separately.
- Chromium quantizes fractional `DelayNode` read positions to 1/256 of a
  sample. The shared delay mirrors that behavior while Tone remains the
  browser reference.
- Integer-sample delay and feedback delay measured around -325 to -336 dB
  relative residual against Tone. Fractional-delay impulses also reached that
  range after quantization was mirrored. Continuous fractional-delay fixtures
  remained around -58 to -62 dB, suggesting an internal block-processing
  detail not described by Tone's public graph. The full chain with a 125 ms
  delay measured about -93 dB on mixed audio.
- Tone's compressor is only a wrapper over native `DynamicsCompressorNode`.
  The shared compressor therefore ports Chromium's current BSD-licensed signal
  model with attribution: 6 ms look-ahead, stereo-linked peak detection,
  32-frame control updates, soft knee, adaptive release, and makeup gain.
  Tone/Chromium versus shared DSP measured about -82 dB on impulse/sweep and
  -84 dB on mixed audio, with maximum error around 2.2e-5. Native and WASM
  compressor output remained bit-identical in the deterministic fixture.
- The shared default chorus reproduces Tone's left-channel impulse taps
  exactly. Continuous left-channel fixtures measured about -58 to -63 dB,
  consistent with the remaining fractional `DelayNode` implementation detail.
  Tone's `OfflineContext` graph emitted no wet right-channel signal in the
  harness, while the shared stereo chorus correctly did; a real-time Tone
  capture is required before using aggregate offline chorus residuals as a
  compatibility score. Native and WASM chorus output is bit-identical.
- Tone.Reverb parameters do not identify its sound: each generation renders
  fresh random noise into a stereo impulse response, then uses native
  `ConvolverNode`. Browser/server parity therefore requires sharing the actual
  IR or defining a deterministic seeded IR. The latter plus partitioned FFT
  convolution is the preferred shared-DSP direction.
- Shared reverb now regenerates a stereo IR from an explicit seed. Its envelope
  follows Tone's pre-delay and exponential-approach shape, including Tone's
  final 10% linear ramp, but energy-normalizes the response deterministically.
  A zero-latency 128-sample direct head preserves early reflections, 128-frame
  partitions cover the next 2,048 samples, and 1,024-frame partitions handle
  the long diffuse tail. Native/WASM output
  and the real AudioWorklet were bit-identical for standalone reverb and the
  all-active effect chain. Exact comparison to Tone's randomly generated IR is
  intentionally meaningless; the migration target is the shared seeded IR.
  The real-browser check uses Tone's 1.5-second default decay, and disabled
  processors now defer IR construction until reverb is enabled.
- Informal listening against the web editor using production clip
  `8161a145-9b3d-4bdb-bca1-4d12fd6781a2` found the native example renders to
  sound as expected, with no difference the listener could identify from the
  web version. This is encouraging perceptual validation, but not yet a
  level-matched blind ABX result.
- The former 30-argument positional setter has been removed. WASM now accepts
  a complete version-2 configuration object and fails closed on unknown
  versions, incomplete fields, non-finite values, or an invalid reverb seed.
- Live `set_effects` updates preserve output continuity and decay the correction
  to the newly configured chain over 5 ms (240 samples at 48 kHz). This avoids
  unsafe coefficient interpolation across nonlinear, delay, compressor,
  chorus, and convolution processors. Static offline renders begin with their
  final configuration and are therefore unchanged.
- Stateful effect parity requires a shared time origin. Letting the worklet
  process silence before a segment started advanced the compressor envelope
  and chorus LFO, causing large differences despite block-independent DSP.
  Deferring/resetting at the fixture's first signal made basic EQ, distortion,
  delay, compressor, chorus, and the complete active chain bit-identical
  between direct WASM and the real-time AudioWorklet. Production must use an
  explicit scheduler start/reset event rather than silence detection, because
  real clips can legitimately begin with silence.
- Tone 15.1.22 is pinned in this standalone harness rather than inherited from
  the production frontend. This keeps Tone available as a comparison oracle
  after its production dependency was removed.
- AudioWorkletGlobalScope does not expose `TextDecoder` in the tested Chromium.
  The versioned object reader makes wasm-bindgen decode JavaScript property
  names, so the worklet now provides a real minimal UTF-8 decoder rather than
  the earlier diagnostic-only placeholder. With that fix, every direct-WASM
  versus AudioWorklet case is again bit-identical.

## Status log

- 2026-08-09: Prototype requested. Current client and server processing chains
  inspected; isolated scope and initial acceptance criteria established.
- 2026-08-09: Native DSP core, tests, raw-PCM utility, WASM boundary, and
  AudioWorklet sketch implemented. Native unit tests pass.
- 2026-08-09: WASM target and web bindings built successfully. The deterministic
  WASM/native fixture is bit-identical.
- 2026-08-09: FFmpeg comparison isolated a shelf-width semantic mismatch in the
  current named filters. Explicit shared biquad coefficients achieved about
  -98 dB relative residual for the combined volume/EQ fixture.
- 2026-08-09: Standalone Chromium harness started. It compares Tone against
  direct WASM separately from the `AudioWorklet` transport so coefficient and
  runtime discrepancies can be diagnosed independently.
- 2026-08-09: Chromium 151 browser measurements completed. Tone and WASM are
  below -96 dB relative residual on all combined fixtures; the real-time
  AudioWorklet capture is bit-identical to direct WASM.
- 2026-08-09: Tone-compatible distortion implemented. Browser diagnostics
  found Tone's effective curve length is 1,024 rather than the constructor's
  apparent 4,096. Correcting that detail improved standalone distortion from
  about -50 dB to -124 dB relative residual on mixed audio.
- 2026-08-09: Feedback delay implemented. Diagnostics isolated Web Audio's
  128-frame cycle break and 1/256-sample fractional read quantization. Integer
  delay/feedback timing now matches Tone at floating-point-noise levels.
- 2026-08-09: Chromium's dynamics compressor signal model ported into Rust and
  exposed through the existing WASM worklet boundary. Initial browser parity
  is below -81 dB on all fixtures; native/WASM output is bit-identical.
- 2026-08-09: Default sine chorus implemented with stereo LFO spread,
  fractional delays, feedback-cycle timing, and wet/dry mixing. Left-channel
  Tone parity validated; the offline Tone right-wet path requires a real-time
  follow-up. Reverb analysis selected deterministic IR plus partitioned
  convolution as the next implementation slice.
- 2026-08-09: Expanded real-time AudioWorklet validation to each stateful
  effect and the complete active chain. Diagnostics found and corrected a
  segment-start lifecycle mismatch; every worklet case is now bit-identical to
  direct WASM for the captured 8,192 frames.
- 2026-08-09: Deterministic seeded stereo reverb implemented with a direct
  convolution head and non-uniform partitioned-FFT tail. Unit, native/WASM,
  and real AudioWorklet block-parity checks pass, including the all-effects
  fixture and Tone's default 1.5-second reverb duration.
- 2026-08-09: Canonical length-changing clip renderer added. Its shared phase
  vocoder/sinc path independently controls duration and pitch; native/WASM
  combined-transform parity measured -117.55 dB. Sine frequency and exact
  duration tests cover current editor limits; speech/transient listening tests
  remain an integration gate.
- 2026-08-09: Frame-safe reverse added at the start of the offline renderer.
  Stereo ordering and reverse-before-rate/pitch tests pass. The combined
  reverse/+700-cent/1.35x native-WASM fixture measured -127.80 dB with the same
  frame count on both targets.
- 2026-08-09: Production clip `8161a145-9b3d-4bdb-bca1-4d12fd6781a2`
  rendered to 24-bit WAV examples for reverb, two pitch directions, independent
  rate, reverse, and the combined effect chain. Production media is ignored by
  Git; preset details and the source checksum live beside the renders.
- 2026-08-09: First real-file listening comparison completed. The listener
  could not distinguish the native renders from the web version and reported
  that the effects sounded as expected. A controlled ABX test remains optional
  before production integration.
- 2026-08-09: Integrated the first seven product controls into the frontend
  through generated WASM and into the server through the native crate. This
  established the shared render path later extended to every implemented DSP
  effect. FFmpeg now handles decode, placement, mix, and encode rather than
  effect semantics for shared-path segments.
- 2026-08-09: Removed Tone.js from the production frontend. Playback now waits
  for the generated WASM asset on first load. It then uses cached shared
  renders and native
  Web Audio scheduling. The full Vite bundle, targeted frontend tests,
  TypeScript, server unit tests, clippy, and a real production-clip server
  render all pass.
- 2026-08-09: Exposed every implemented advanced DSP parameter in collapsible
  frontend inspector groups and carried them through composition JSON into the
  native server renderer. Rapid slider changes are coalesced before expensive
  WASM re-rendering. A real production clip rendered successfully with
  distortion, feedback delay, compressor, chorus, and deterministic reverb
  active together. Advanced effects are temporarily limited to source windows
  of 60 seconds while the offline renderer remains in-memory.
- 2026-08-09: Added the monorepo root to Vite's development file allow-list.
  The exact sibling `sakiot_dsp_bg.wasm` request now returns HTTP 200 as
  `application/wasm`; production continues to use the copied `dist/assets`
  artifact.
- 2026-08-09: Documented the fixed DSP-owned effect order. Inspector editing
  order is not signal-chain order, and compositions store values rather than
  control-change history. A reorderable chain is a separate future data-model
  and execution-contract change shared by browser and server.
- 2026-08-09: Replaced the 30-argument WASM call with a fail-closed versioned
  effect object and added deterministic 5 ms output-continuity smoothing for
  live native/WASM parameter changes. Client timeline waveforms now come from
  the exact cached effect-processed PCM used for playback and update after a
  120 ms edit debounce; the server source waveform remains the fallback.
- 2026-08-09: Moved the pinned Tone 15.1.22 comparison dependency into the
  standalone DSP harness and repaired its AudioWorklet UTF-8 fallback for the
  object-config boundary. The real Chromium harness passes again, with every
  worklet case bit-identical to direct WASM.
- 2026-08-09: Removed the waveform fallback flash during effect edits. Once a
  segment has processed peaks, the editor retains them until the next WASM
  result is ready; reverse changes optimistically mirror those processed peaks
  instead of briefly showing the reversed raw-source envelope.
- 2026-08-09: Implemented the explicit effect-tail policy end to end. A 0-30s
  tail is appended after reverse/pitch/rate and processed by the shared chain;
  browser PCM/playback/waveforms, timeline editing and splitting, composition
  JSON, validation, native rendering, and duration accounting now agree.
- 2026-08-09: Added a hidden `Ctrl+Shift+O` effect-settings JSON editor. It
  validates complete or partial camelCase DSP settings and applies them to the
  current selection through the same duration-aware editor model as inspector
  controls. The combined-chain example is now strict, paste-ready JSON.
- 2026-08-09: Added a dedicated DSP CI scope and job. It runs standalone
  formatting/clippy/tests, rebuilds WASM and the worklet with pinned tooling,
  verifies native/WASM parity, and rejects stale committed browser artifacts.
