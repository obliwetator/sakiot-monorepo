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

	test("pitch does not change buffer consumption", () => {
		const pitched = seg(1, 0, 10, 0);
		pitched.effects.pitchCents = 1200;
		const { offset, duration } = segmentSourceWindow(pitched, 0, 0, 10);
		expect(offset).toBe(0);
		expect(duration).toBe(10);
	});

	test("pitch does not compound with speed when seeking", () => {
		const pitched = seg(2, 0, 10, 0);
		pitched.effects.pitchCents = 1200;
		const { offset, duration } = segmentSourceWindow(pitched, 2, 2, 3);
		expect(offset).toBe(4);
		expect(duration).toBe(2);
	});

	test("reversed segments start at the source-window end", () => {
		const reversed = seg(1, 0, 10, 0);
		reversed.effects.reverse = true;
		const { offset, duration } = segmentSourceWindow(reversed, 0, 0, 10);
		expect(offset).toBe(10);
		expect(duration).toBe(10);
	});

	test("reversed segments walk the window backwards when seeking", () => {
		const reversed = seg(2, 0, 10, 0);
		reversed.effects.reverse = true;
		// Two seconds into a 2x reversed segment plays the content that was
		// two seconds before the window end, i.e. buffer second 6.
		const { offset, duration } = segmentSourceWindow(reversed, 2, 2, 5);
		expect(offset).toBe(6);
		expect(duration).toBe(6);
	});

	test("reversed segments respect the source-in offset", () => {
		const reversed = seg(1, 2, 8, 0);
		reversed.effects.reverse = true;
		const { offset, duration } = segmentSourceWindow(reversed, 0, 0, 6);
		expect(offset).toBe(8);
		expect(duration).toBe(6);
	});

	test("the emergency fallback never reads source audio during a tail", () => {
		const tailed = seg(1, 0, 10, 0);
		tailed.effects.tailSeconds = 2;
		expect(segmentSourceWindow(tailed, 0, 8, 12)).toEqual({
			offset: 8,
			duration: 2,
		});
		expect(segmentSourceWindow(tailed, 0, 10, 12)).toEqual({
			offset: 10,
			duration: 0,
		});
	});
});
