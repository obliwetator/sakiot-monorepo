import { describe, expect, test } from "bun:test";
import { decodeWaveformPeaks } from "./waveformPeaks";

function waveformData(flags: number, values: number[], sixteenBit: boolean) {
	const bytes = new Uint8Array(20 + values.length * (sixteenBit ? 2 : 1));
	const view = new DataView(bytes.buffer);
	view.setUint32(4, flags, true);
	view.setUint32(16, values.length, true);
	values.forEach((value, index) => {
		if (sixteenBit) {
			view.setInt16(20 + index * 2, value, true);
		} else {
			view.setInt8(20 + index, value);
		}
	});
	return btoa(String.fromCharCode(...bytes));
}

describe("decodeWaveformPeaks", () => {
	test("normalizes 8-bit waveform samples", () => {
		expect(decodeWaveformPeaks(waveformData(0, [-128, 0, 127], false))).toEqual(
			[-1, 0, 127 / 128],
		);
	});

	test("normalizes 16-bit waveform samples", () => {
		expect(
			decodeWaveformPeaks(waveformData(1, [-32768, 0, 32767], true)),
		).toEqual([-1, 0, 32767 / 32768]);
	});

	test("returns no peaks for an incomplete header", () => {
		expect(decodeWaveformPeaks(btoa("short"))).toEqual([]);
	});
});
