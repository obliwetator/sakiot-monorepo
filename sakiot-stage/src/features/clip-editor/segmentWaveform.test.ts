import { describe, expect, test } from "bun:test";
import {
	segmentWaveformStrokeStyle,
	waveformWindowFractions,
} from "./SegmentWaveform";

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

describe("segmentWaveformStrokeStyle", () => {
	test("uses a colored stroke for audible clips", () => {
		expect(segmentWaveformStrokeStyle(false, false)).toBe(
			"rgba(45, 212, 191, 0.9)",
		);
		expect(segmentWaveformStrokeStyle(true, false)).toBe(
			"rgba(165, 243, 252, 0.95)",
		);
	});

	test("uses gray tones for muted clips", () => {
		expect(segmentWaveformStrokeStyle(false, true)).toBe(
			"rgba(148, 163, 184, 0.72)",
		);
		expect(segmentWaveformStrokeStyle(true, true)).toBe(
			"rgba(203, 213, 225, 0.9)",
		);
	});
});
