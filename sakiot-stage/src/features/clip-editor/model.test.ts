import { describe, expect, test } from "bun:test";
import {
	DEFAULT_EFFECTS,
	segmentDuration,
	setSegmentSpeed,
	snapToNeighbors,
	type TimelineSegment,
} from "./model";

function seg(id: string, track: number, start: number, duration: number) {
	const segment: TimelineSegment = {
		id,
		track,
		source: "clip",
		sourceId: id,
		sourceIn: 0,
		sourceOut: duration,
		timelineStart: start,
		effects: { ...DEFAULT_EFFECTS },
	};
	return segment;
}

const neighbor = seg("n", 0, 10, 4);

function editWith(...segments: TimelineSegment[]) {
	return {
		segments,
		tracks: 1,
		masterVolumeDb: 0,
	};
}

describe("snapToNeighbors", () => {
	test("leaves a non-overlapping start alone", () => {
		expect(snapToNeighbors(4, 2, [neighbor], "x", 0, 4)).toBe(4);
	});

	test("touching the neighbour's start is not an overlap", () => {
		expect(snapToNeighbors(6, 2, [neighbor], "x", 0, 6)).toBe(6);
	});

	test("cursor over the left half snaps the end to the neighbour's start", () => {
		expect(snapToNeighbors(8, 4, [neighbor], "x", 0, 8)).toBe(6);
	});

	test("cursor over the right half snaps the start to the neighbour's end", () => {
		expect(snapToNeighbors(12, 2, [neighbor], "x", 0, 12)).toBe(14);
	});

	test("the dragged segment itself is excluded", () => {
		const self = seg("x", 0, 8, 4);
		expect(snapToNeighbors(8, 4, [self], "x", 0, 8)).toBe(8);
	});

	test("neighbours on other tracks are ignored", () => {
		const other = seg("o", 1, 10, 4);
		expect(snapToNeighbors(10, 4, [other], "x", 0, 10)).toBe(10);
	});

	test("prefers a non-overlapping candidate over a clamped overlap", () => {
		const early = seg("e", 0, 2, 2);
		expect(snapToNeighbors(0, 5, [early], "x", 0, 0)).toBe(4);
	});

	test("the neighbour under the cursor wins when several overlap", () => {
		const left = seg("l", 0, 2, 4);
		const right = seg("r", 0, 12, 4);
		const segments = [left, right];
		expect(snapToNeighbors(4, 10, segments, "x", 0, 4)).toBe(16);
		expect(snapToNeighbors(9, 10, segments, "x", 0, 9)).toBe(16);
	});

	test("adjacent clips cannot trap a third clip between them", () => {
		const a = seg("a", 0, 2, 4);
		const b = seg("b", 0, 6, 4);
		expect(snapToNeighbors(4, 3, [a, b], "x", 0, 4)).toBe(10);
		expect(snapToNeighbors(7, 3, [a, b], "x", 0, 7)).toBe(10);
	});

	test("a clip too long for the gap resolves to the free side", () => {
		const a = seg("a", 0, 2, 4);
		const b = seg("b", 0, 8, 4);
		expect(snapToNeighbors(5, 4, [a, b], "x", 0, 5)).toBe(12);
	});

	test("four chained clips cannot trap a fifth between them", () => {
		const chain = [
			seg("a", 0, 0, 4),
			seg("b", 0, 4, 4),
			seg("c", 0, 8, 4),
			seg("d", 0, 12, 4),
		];
		expect(snapToNeighbors(5, 3, chain, "x", 0, 5)).toBe(16);
		expect(snapToNeighbors(7, 3, chain, "x", 0, 7)).toBe(16);
		expect(snapToNeighbors(9, 3, chain, "x", 0, 9)).toBe(16);
	});

	test("a gap in the chain stays usable", () => {
		const chain = [
			seg("a", 0, 0, 4),
			seg("b", 0, 4, 4),
			seg("c", 0, 10, 4),
			seg("d", 0, 14, 4),
		];
		expect(snapToNeighbors(6, 2, chain, "x", 0, 6)).toBe(8);
		expect(snapToNeighbors(5, 3, chain, "x", 0, 5)).toBe(18);
		expect(snapToNeighbors(5, 4, chain, "x", 0, 5)).toBe(18);
	});

	test("exact fit between two neighbours is preserved", () => {
		const left = seg("l", 0, 2, 4);
		const right = seg("r", 0, 12, 4);
		expect(snapToNeighbors(6, 6, [left, right], "x", 0, 6)).toBe(6);
	});
});

describe("setSegmentSpeed", () => {
	test("speeding up shrinks the segment and leaves neighbours alone", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 4, 4);
		const next = setSegmentSpeed(editWith(a, b), "a", 2);
		expect(next.segments[0]?.effects.rate).toBe(2);
		expect(segmentDuration(next.segments[0] as TimelineSegment)).toBe(2);
		expect(next.segments[1]?.timelineStart).toBe(4);
	});

	test("slowing down extends and pushes the snapped neighbour right", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 4, 4);
		const next = setSegmentSpeed(editWith(a, b), "a", 0.5);
		expect(segmentDuration(next.segments[0] as TimelineSegment)).toBe(8);
		expect(next.segments[1]?.timelineStart).toBe(8);
	});

	test("an extension into a gap pushes a neighbour it reaches", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 8, 4);
		const next = setSegmentSpeed(editWith(a, b), "a", 0.5);
		expect(segmentDuration(next.segments[0] as TimelineSegment)).toBe(8);
		expect(next.segments[1]?.timelineStart).toBe(8);
	});

	test("a small extension into a gap does not move the neighbour", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 6, 4);
		const next = setSegmentSpeed(editWith(a, b), "a", 2 / 3);
		expect(segmentDuration(next.segments[0] as TimelineSegment)).toBe(6);
		expect(next.segments[1]?.timelineStart).toBe(6);
	});

	test("a chained suffix is pushed as a whole", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 4, 4);
		const c = seg("c", 0, 8, 4);
		const next = setSegmentSpeed(editWith(a, b, c), "a", 0.5);
		expect(next.segments[1]?.timelineStart).toBe(8);
		expect(next.segments[2]?.timelineStart).toBe(12);
	});

	test("contraction never pulls a snapped neighbour", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 4, 4);
		const next = setSegmentSpeed(editWith(a, b), "a", 4);
		expect(next.segments[1]?.timelineStart).toBe(4);
	});

	test("a bad rate is rejected", () => {
		const a = seg("a", 0, 0, 4);
		const edit = editWith(a);
		expect(setSegmentSpeed(edit, "a", 0)).toBe(edit);
	});
});
