import { describe, expect, test } from "bun:test";
import { decodeWaveformPeaks } from "./waveformPeaks";

/**
 * Builds a payload the way audiowaveform writes one: `length` counts min/max
 * pairs, and the flag is set for 8-bit output. Verified against the fixtures in
 * the audiowaveform test corpus, where the 8-bit file carries flags=1 with two
 * bytes per point and the 16-bit file carries flags=0 with four.
 */
function waveformData(pairs: ReadonlyArray<[number, number]>, bits: 8 | 16) {
	const bytesPerValue = bits === 8 ? 1 : 2;
	const bytes = new Uint8Array(20 + pairs.length * 2 * bytesPerValue);
	const view = new DataView(bytes.buffer);
	view.setInt32(0, 1, true);
	view.setUint32(4, bits === 8 ? 1 : 0, true);
	view.setInt32(8, 16_000, true);
	view.setInt32(12, 64, true);
	view.setUint32(16, pairs.length, true);
	pairs.forEach(([min, max], index) => {
		const offset = 20 + index * 2 * bytesPerValue;
		if (bits === 8) {
			view.setInt8(offset, min);
			view.setInt8(offset + 1, max);
		} else {
			view.setInt16(offset, min, true);
			view.setInt16(offset + 2, max, true);
		}
	});
	return btoa(String.fromCharCode(...bytes));
}

describe("decodeWaveformPeaks", () => {
	test("normalizes 8-bit waveform samples", () => {
		expect(
			decodeWaveformPeaks(
				waveformData(
					[
						[-128, 127],
						[-64, 64],
						[0, 0],
					],
					8,
				),
			),
		).toEqual({
			min: [-1, -0.5, 0],
			max: [127 / 128, 0.5, 0],
			durationMs: 12,
		});
	});

	test("normalizes 16-bit waveform samples", () => {
		expect(
			decodeWaveformPeaks(
				waveformData(
					[
						[-32768, 32767],
						[-16384, 16384],
					],
					16,
				),
			),
		).toEqual({
			min: [-1, -0.5],
			max: [32767 / 32768, 0.5],
			durationMs: 8,
		});
	});

	test("decodes every point instead of half the recording", () => {
		const pairs = Array.from(
			{ length: 500 },
			(_value, index) => [-index, index] as [number, number],
		);

		const peaks = decodeWaveformPeaks(waveformData(pairs, 16));

		expect(peaks.min).toHaveLength(500);
		expect(peaks.max[499]).toBeCloseTo(499 / 32768, 10);
	});

	test("stops at a truncated point rather than reading past the end", () => {
		const complete = atob(
			waveformData(
				[
					[-64, 64],
					[-32, 32],
				],
				8,
			),
		);

		expect(
			decodeWaveformPeaks(btoa(complete.slice(0, complete.length - 1))),
		).toEqual({ min: [-0.5], max: [0.5], durationMs: 4 });
	});

	test("returns no peaks for an incomplete header", () => {
		expect(decodeWaveformPeaks(btoa("short"))).toEqual({ min: [], max: [] });
	});
});
