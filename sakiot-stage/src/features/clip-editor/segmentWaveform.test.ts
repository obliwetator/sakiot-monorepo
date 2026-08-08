import { describe, expect, test } from "bun:test";
import { waveformWindowFractions } from "./SegmentWaveform";

describe("waveformWindowFractions", () => {
	test("maps the source window onto the clip duration", () => {
		expect(waveformWindowFractions(1, 5, 10)).toEqual({
			startFraction: 0.1,
			endFraction: 0.5,
		});
	});

	test("clamps out-of-range edges", () => {
		expect(waveformWindowFractions(-2, 20, 10)).toEqual({
			startFraction: 0,
			endFraction: 1,
		});
	});

	test("is null when the window is empty or duration is unusable", () => {
		expect(waveformWindowFractions(5, 5, 10)).toBeNull();
		expect(waveformWindowFractions(0, 5, 0)).toBeNull();
		expect(waveformWindowFractions(0, 5, Number.NaN)).toBeNull();
		expect(waveformWindowFractions(0, 5, -1)).toBeNull();
	});

	test("treats a full-length window as the whole clip", () => {
		expect(waveformWindowFractions(0, 10, 10)).toEqual({
			startFraction: 0,
			endFraction: 1,
		});
	});
});
