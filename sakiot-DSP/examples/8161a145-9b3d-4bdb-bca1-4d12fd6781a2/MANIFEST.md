# Real-file DSP examples

Source production clip: `8161a145-9b3d-4bdb-bca1-4d12fd6781a2`

All listening files are 48 kHz stereo, 24-bit PCM WAV. The media files in this
directory are ignored by Git so production audio cannot be committed by
accident.

| File | Shared DSP settings |
| --- | --- |
| `01_original.wav` | Reference decode with no DSP. |
| `02_reverb.wav` | Seeded reverb: 1.5 s decay, 10 ms pre-delay, 50% wet. Includes 1.6 s of tail. |
| `03_pitch_up_700c.wav` | Pitch +700 cents, unchanged duration. |
| `04_pitch_down_500c.wav` | Pitch -500 cents, unchanged duration. |
| `05_rate_1.35x.wav` | 1.35x playback rate with pitch held constant. |
| `06_reverse.wav` | Complete stereo frames reversed before all other processing. |
| `07_combined_chain.wav` | Pitch +400 cents, rate 0.85x, volume -3 dB, EQ +4/-2/+2 dB, distortion 35% wet, feedback delay 25% wet, compressor, chorus 35% wet, and seeded reverb 30% wet. Set Effect tail to 2.0 s; total duration is 14.227458 s. |

`07_combined_chain.wav` is now reproducible through the editor: select the
segment, press `Ctrl+Shift+O`, and paste the complete strict JSON object from
`generation.txt`. It includes `tailSeconds: 2`. Tail duration is fixed after
rate processing, so the 2 seconds are not stretched by the 0.85x rate.

The deterministic reverb seed is `0x53414b49` (`1396788041`).

## Source checksum

```text
SHA-256 b7a21ccaaa11fd9cd797fa3ce762bdf1fcd8665922c431a42e4aeac992ba99c9  source.ogg
```
