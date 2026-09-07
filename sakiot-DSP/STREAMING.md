# Incremental composition rendering

Renderer version 2 uses shared Rust DSP for every server segment, independently
of source duration and pitch/rate stretch. FFmpeg decodes source windows to
stereo 48 kHz float32 PCM, then places, mixes and Opus-encodes the processed PCM.
The server no longer has an FFmpeg/Rubber Band effects fallback. Placement
concatenates generated leading silence instead of allocating an `adelay` ring
proportional to a segment's timeline start.

`IncrementalRenderer::new(rate, channels, input_frames, effects)` declares the
exact decoded source length. `push(block, sink)` accepts arbitrary aligned input
blocks and calls the sink with at most 512 output frames. `finish(sink)` drains
the remaining overlap/resampling output and appends the configured effect tail
once. Incomplete/excess input, misaligned samples and repeated finalization are
errors. A sink failure makes the renderer terminal; discard that attempt.

The renderer retains phase-vocoder analysis history, previous/synthesis phases,
overlap and normalization, the sinc resampler's source history and fractional
position, and one `SegmentProcessor` for the complete segment. Compressor,
delay, chorus, reverb, EQ and smoothing state persist across calls. Input is
internally consumed in 512-frame blocks, even if callers supply a large slice.
FFT size is 2048, synthesis hop 512, and resampling radius 24. Retained geometry
storage depends on these constants and the validated pitch/rate parameters,
not on segment length. Effect storage depends on configured delay/reverb times.

Reverse requires ordered input: the API rejects `reverse: true`. The server
reads a seekable decoded PCM file backwards in 4096-frame blocks, reverses whole
stereo frames inside each block, and passes `reverse: false` to the renderer.
The worker does the equivalent traversal of its source arrays. Channel order
is never reversed. Forward rendering also uses decoded temporary PCM to obtain
an exact frame count before processing, avoiding codec-duration estimates.

All server PCM files live inside the existing job attempt directory. Decoding
writes directly to disk; processing reads/writes blocks. Only one segment is
processed at a time. Temporary decoded input is removed after each segment,
while processed PCM remains until the common mix completes. The existing
process group, lease fencing, recovery, atomic clip/archive publication,
30-minute attempt timeout, 2 GiB address-space limit and 8 GiB disk budget are
unchanged. Disk use includes the decoded current segment and all processed
segments; sufficiently large or heavily overlapping compositions can still
exhaust that budget and fail explicitly. Incremental rendering does not make
CPU time or disk usage independent of duration.

The whole-buffer renderer remains a numerical reference. Both implementations
now synthesize enough trailing overlap for extreme stretches: the previous
crop could truncate output or panic on very short sources. Both use f64 absolute
source positions, avoiding loss of sub-frame precision on long recordings, and
the same pinned libm calculations for pitch, FFT, resampling and stateful effects.
Platform-specific float math previously accumulated into phase and compressor
differences on long inputs and at 160x stretch.
These are renderer-version changes, not claims of byte identity with version 1.

# Persisted jobs

New jobs store renderer version 2. Under the existing queue lock, the version 2
worker explicitly migrates queued version 1 jobs and expired version 1 attempts
to version 2 before claiming them. Active version 1 leases are left alone, so an
already-running old worker can finish using its original renderer. Expired
attempts get a new fencing token; the old attempt cannot renew or publish.
Terminal results and future/unknown renderer versions are not rewritten. Tests
cover queued migration, preservation of active old leases, expired migration,
and stale-token rejection in addition to the existing crash-recovery tests.

# Browser limits

Browser preprocessing uses `WasmIncrementalRenderer` with 4096-frame blocks.
Interleaving and WASM pitch/rate intermediates are bounded; effect processing
also copies blocks through one persistent WASM processor. The final JavaScript
PCM array is allocated once, and processed waveforms still derive from that
exact PCM. The existing AudioWorklet processes the live effect suffix.

Decoded-source cache ownership is capped at 64 MiB, using LRU eviction and
serialized fetching/decoding. Processed PCM, waveform values and cached Web Audio
copies share a separate 64 MiB LRU budget. These budgets describe retained PCM
payloads, not the browser's entire heap. Cache eviction does not invalidate
buffers currently used by playback or the editor.

**Long browser editing is still constrained.** `decodeAudioData` decodes a whole
clip; source buffers held by editor state, queued transfers and active playback
are outside those cache budgets. A trimmed segment can therefore still require
decoding a much longer original clip. The browser keeps complete final PCM and
Web Audio buffers, and copies source windows to its worker. This is not a fully
streaming decoder/playback system or a guarantee against browser OOM on huge
source files. A future windowed decode service or seekable browser decoder and
streamed playback would be required for that guarantee.

Preview is explicitly disabled with an explanatory notice when an unmuted
segment's source window or rendered output exceeds 64 MiB of stereo PCM (about
175 seconds at 48 kHz), or total unmuted rendered PCM exceeds 128 MiB. Rate and
tail are included. Keyboard playback observes the same limit. Server export
remains available. Oversized waveform/preprocess requests are excluded before
copying source windows; their source/last-processed envelopes remain available.

# Verification and measurements

`tests/incremental.rs` compares arbitrary block sizes (including one frame),
short/empty input, fractional rate/pitch, the full stateful chain, reverse and
tails against the whole-buffer reference. Native/reference tolerances are
maximum absolute error <= 2e-6 and RMS error <= 2e-7. The native/WASM check uses
maximum absolute error <= 2e-6 and RMS error <= 2e-7,
including the full chain at +4800 cents / 0.1 rate and -4800 cents / 10 rate.
The checks also cover a 61-second stereo source with the full chain. All tested
final native/WASM outputs were bit-exact (zero maximum and RMS error).
The real AudioWorklet harness also checks its output against direct WASM.

Server regressions exercise the seekable reverse adapter and generated stereo
media across 59.99, 60.00 and 60.01 seconds. Adding those long segments leaves the
short effected segment's PCM byte-identical, and common encoding produces stereo
48 kHz Opus with the expected duration. Playwright tests cover the preview notice,
keyboard behavior and export availability on desktop and mobile.

Release-build DSP measurements on the development Linux host (2026-09-06), with
stereo 48 kHz input, distortion, delay, compressor, chorus, seeded 1.5-second
reverb and a 0.5-second tail:

| Source duration | Pitch / rate | Peak RSS | Wall time |
| --- | --- | --- | --- |
| 60 seconds | +700 cents / 1.35 | 8,140 KiB | 9.97 seconds |
| 600 seconds | +700 cents / 1.35 | 8,016 KiB | 112.76 seconds |
| 6 seconds | +4800 cents / 0.1 (160x intermediate stretch) | 8,028 KiB | 31.02 seconds |

A separate maximum-effect-storage run (60-second source, no pitch/rate,
5-second delay, 100 ms chorus delay, 30-second reverb with 5-second pre-delay,
and a 30-second tail) used 123,384 KiB peak RSS and 35.05 seconds wall time.

A separate FFmpeg placement/Opus check placed 0.2 seconds of audio after 3599.8
seconds of generated silence. It used 52,716 KiB peak RSS and 13.18 seconds wall
time. A PCM regression checks the common mix against the expected shifted sum
to 1e-9 absolute error, including quiet stereo tails below 16-bit resolution.

These measurements consume output immediately rather than retaining it. They
measure the DSP component, excluding FFmpeg, the web process, disk I/O and
browser allocations. The roughly constant RSS establishes duration-independent
DSP memory for this configuration; it is not an upper bound for all supported
reverb/delay settings. Extreme pitch/rate increases work even though memory
remains bounded. The existing job limits remain the final resource guard.

Reproduce from the repository root:

```sh
cargo test --manifest-path sakiot-DSP/Cargo.toml --locked --release
cargo build --manifest-path sakiot-DSP/Cargo.toml --locked --release --example measure_incremental
/usr/bin/time -v sakiot-DSP/target/release/examples/measure_incremental 60 700 1.35
/usr/bin/time -v sakiot-DSP/target/release/examples/measure_incremental 600 700 1.35
/usr/bin/time -v sakiot-DSP/target/release/examples/measure_incremental 6 4800 0.1
/usr/bin/time -v sakiot-DSP/target/release/examples/measure_incremental 60 0 1 max-effects
node sakiot-DSP/web/verify-wasm.mjs
```

Final verification: 335 workspace Rust tests (one existing manual-media test
ignored), 26 DSP tests (one existing manual FFmpeg EQ measurement ignored),
383 frontend tests, and 31 Playwright tests across desktop/mobile (three
viewport-specific skips). Build, workspace/DSP Clippy, formatting, SQLx query
metadata, native/WASM parity, real AudioWorklet checks and job crash recovery
passed. Database tests used a disposable local PostgreSQL container.
