import { describe, expect, test } from "bun:test";
import { DEFAULT_EFFECTS, type TimelineSegment } from "./model";
import {
	type GroupedSegment,
	groupTrackCollision,
	resolveGroupDelta,
} from "./Timeline";

function member(id: string, originStart: number, duration = 4): GroupedSegment {
	return { id, originStart, originTrack: 0, duration };
}

function segment(id: string, start: number, duration = 4): TimelineSegment {
	return {
		id,
		track: 0,
		source: "clip",
		sourceId: id,
		sourceIn: 0,
		sourceOut: duration,
		timelineStart: start,
		effects: { ...DEFAULT_EFFECTS },
	};
}

describe("resolveGroupDelta", () => {
	test("moves every member by the target delta when nothing blocks", () => {
		const group = [member("a", 0), member("b", 4)];
		const delta = resolveGroupDelta(group, 2, 2, 0, []);
		expect(delta).toBe(2);
	});

	test("never lets a member drop below zero", () => {
		const group = [member("a", 0), member("b", 4)];
		expect(resolveGroupDelta(group, 2, -3, 0, [])).toBe(0);
	});

	test("clamps at the left edge with the leftmost member touching zero", () => {
		const group = [member("a", 4), member("b", 8)];
		// Dragging far left would push A before 0; the group stops with A
		// exactly at 0 and B keeping its offset.
		expect(resolveGroupDelta(group, 2, -5, 0, [])).toBe(-4);
	});

	test("stops the whole group at the first blocked member", () => {
		const group = [member("a", 0), member("b", 4)];
		const obstacle = segment("c", 10, 2);
		// B (end 8 + delta) would overlap C for deltas in (2, 8); the
		// nearest valid stop is B touching C's start, moving the group by 2.
		const delta = resolveGroupDelta(group, 2, 6, 0, [obstacle]);
		expect(delta).toBe(2);
	});

	test("snaps to the near side of the obstacle when the target is inside it", () => {
		const group = [member("a", 0)];
		const obstacle = segment("c", 10, 2);
		// The forbidden deltas are (6, 12); the target 7 lands inside, so the
		// nearest valid delta is 6 (the segment touching C's start).
		expect(resolveGroupDelta(group, 2, 7, 0, [obstacle])).toBe(6);
	});

	test("ignores obstacles on other tracks", () => {
		const group = [member("a", 0), member("b", 4)];
		const otherTrack = { ...segment("c", 10, 2), track: 1 };
		expect(resolveGroupDelta(group, 2, 6, 0, [otherTrack])).toBe(6);
	});

	test("keeps the group rigid when dragging left against an obstacle", () => {
		const group = [member("a", 6), member("b", 10)];
		const obstacle = segment("c", 0, 4);
		// A (start 6) is blocked by C's end 4: the nearest valid delta is
		// 4 - 6 = -2, so both members move left together.
		const delta = resolveGroupDelta(group, 2, -4, 0, [obstacle]);
		expect(delta).toBe(-2);
	});
});

describe("groupTrackCollision", () => {
	const onTrack = (id: string, track: number, start: number) => ({
		...member(id, start),
		originTrack: track,
	});

	test("moving up that squashes two tracks together collides", () => {
		const group = [onTrack("a", 0, 0), onTrack("b", 1, 0), onTrack("c", 2, 0)];
		// Track 0's member clamps at track 0 while track 1's member lands on
		// track 0 too; both overlap in time.
		expect(groupTrackCollision(group, -1)).toBe(true);
	});

	test("moving down never collides even over several tracks", () => {
		const group = [onTrack("a", 0, 0), onTrack("b", 1, 0), onTrack("c", 2, 0)];
		expect(groupTrackCollision(group, 1)).toBe(false);
		// Moving down by two: c would land on a brand-new track; the upper
		// clamp must never squash it onto the first phantom track.
		expect(groupTrackCollision(group, 2)).toBe(false);
	});

	test("members sharing a track are fine when their times do not overlap", () => {
		const group = [onTrack("a", 0, 0), onTrack("b", 0, 8), onTrack("c", 2, 0)];
		// a and b both clamp onto track 0, but they play at different times;
		// c lands on track 1, so nothing overlaps.
		expect(groupTrackCollision(group, -1)).toBe(false);
	});

	test("no vertical movement never collides", () => {
		const group = [onTrack("a", 0, 0), onTrack("b", 1, 0)];
		expect(groupTrackCollision(group, 0)).toBe(false);
	});
});
