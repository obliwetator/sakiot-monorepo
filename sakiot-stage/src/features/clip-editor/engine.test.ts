import { describe, expect, test } from "bun:test";
import { segmentSourceWindow } from "./engine";
import { DEFAULT_EFFECTS, type TimelineSegment } from "./model";

function seg(rate: number, sourceIn = 0, sourceOut = 10, timelineStart = 0) {
	const segment: TimelineSegment = {
		id: "seg",
		track: 0,
		source: "clip",
		sourceId: "clip-1",
		sourceIn,
		sourceOut,
		timelineStart,
		effects: { ...DEFAULT_EFFECTS, rate },
	};
	return segment;
}

describe("segmentSourceWindow", () => {
	test("rate 1 keeps the buffer window equal to the timeline window", () => {
		const { offset, duration } = segmentSourceWindow(seg(1), 0, 0, 10);
		expect(offset).toBe(0);
		expect(duration).toBe(10);
	});

	test("faster segments consume more buffer content per timeline second", () => {
		// 10s of content at 2x plays inside a 5s window.
		const { offset, duration } = segmentSourceWindow(seg(2), 0, 0, 5);
		expect(offset).toBe(0);
		expect(duration).toBe(10);
	});

	test("slower segments consume less buffer content per timeline second", () => {
		// 6s of trimmed content at 0.5x fills a 12s window starting at sourceIn=2.
		const { offset, duration } = segmentSourceWindow(seg(0.5, 2, 8), 0, 0, 12);
		expect(offset).toBe(2);
		expect(duration).toBe(6);
	});

	test("seek offsets are rate-adjusted", () => {
		// Two seconds into a 2x segment lands at buffer second 4, and the
		// remaining 3s of window need 6 buffer seconds.
		const { offset, duration } = segmentSourceWindow(seg(2), 2, 2, 5);
		expect(offset).toBe(4);
		expect(duration).toBe(6);
	});

	test("a clip offset in the source does not change the window width", () => {
		const { offset, duration } = segmentSourceWindow(seg(2, 3, 9), 0, 0, 3);
		expect(offset).toBe(3);
		expect(duration).toBe(6);
	});

	test("pitch up doubles the buffer consumption like the speed control", () => {
		// +1200 cents: effective rate 2, so 10s of content fits a 5s window.
		const pitched = seg(1, 0, 10, 0);
		pitched.effects.pitchCents = 1200;
		const { offset, duration } = segmentSourceWindow(pitched, 0, 0, 5);
		expect(offset).toBe(0);
		expect(duration).toBe(10);
	});

	test("pitch down halves the buffer consumption", () => {
		// -1200 cents: effective rate 0.5, so 6s of content fills a 12s window.
		const pitched = seg(1, 2, 8, 0);
		pitched.effects.pitchCents = -1200;
		const { offset, duration } = segmentSourceWindow(pitched, 0, 0, 12);
		expect(offset).toBe(2);
		expect(duration).toBe(6);
	});

	test("pitch compounds with the speed when seeking", () => {
		// rate 2 + pitch +1200: effective rate 4, so 2s in lands at buffer 8.
		const pitched = seg(2, 0, 10, 0);
		pitched.effects.pitchCents = 1200;
		const { offset, duration } = segmentSourceWindow(pitched, 2, 2, 3);
		expect(offset).toBe(8);
		expect(duration).toBe(4);
	});
});
