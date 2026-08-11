import { describe, expect, test } from "bun:test";
import { DEFAULT_EFFECTS, type TimelineSegment } from "./model";
import {
	applySegmentDrag,
	clampPointToRect,
	type GroupedSegment,
	groupTrackCollision,
	marqueeIntersectsSegment,
	marqueeOverlayOffset,
	resolveGroupDelta,
	type SegmentDragMode,
	type SegmentDragState,
	timelineScrollRequest,
	transitionTimelineDrag,
} from "./timelineDrag";

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

function dragState(
	mode: SegmentDragMode,
	overrides: Partial<SegmentDragState> = {},
): SegmentDragState {
	return {
		mode,
		segmentId: "a",
		group: [member("a", 10, 6)],
		originStart: 10,
		originIn: 2,
		originOut: 8,
		originTrack: 0,
		originRate: 1,
		originTail: 0,
		reverse: false,
		maxSource: 10,
		maxTrack: 1,
		startX: 100,
		startY: 50,
		ghostStart: 10,
		ghostIn: 2,
		ghostOut: 8,
		ghostTrack: 0,
		ghostStarts: [],
		modifierClick: false,
		trackCollision: false,
		valid: true,
		clamped: false,
		pointerX: 100,
		pointerY: 50,
		...overrides,
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

describe("timeline drag transitions", () => {
	const pointer = (clientX: number, clientY = 50) => ({
		clientX,
		clientY,
		containerRect: { top: 0, bottom: 100, height: 100 },
	});

	test("reverse left resize extends sourceOut and moves timeline start left", () => {
		const result = transitionTimelineDrag(
			dragState("left", { reverse: true }),
			pointer(80),
			1,
			100,
			[segment("a", 10, 6)],
			0,
		);
		expect(result.state.ghostOut).toBe(10);
		expect(result.state.ghostIn).toBe(2);
		expect(result.state.ghostStart).toBe(8);
	});

	test("reverse right resize extends toward source start", () => {
		const result = transitionTimelineDrag(
			dragState("right", { reverse: true }),
			pointer(120),
			1,
			100,
			[segment("a", 10, 6)],
			0,
		);
		expect(result.state.ghostIn).toBe(0);
		expect(result.state.ghostOut).toBe(8);
	});

	test("rejects vertical group move that merges overlapping tracks", () => {
		const group = [
			{ ...member("a", 0), originTrack: 0 },
			{ ...member("b", 0), originTrack: 1 },
		];
		const result = transitionTimelineDrag(
			dragState("move", {
				segmentId: "b",
				group,
				originStart: 0,
				originTrack: 1,
				ghostStart: 0,
				ghostTrack: 1,
			}),
			pointer(110),
			1,
			100,
			[segment("a", 0), { ...segment("b", 0), track: 1 }],
			0,
		);
		expect(result.state.trackCollision).toBe(true);
		expect(result.state.ghostStarts).toEqual([
			{ id: "a", start: 0 },
			{ id: "b", start: 0 },
		]);
	});

	test("commits every group member and grows track count", () => {
		const a = segment("a", 0);
		const b = { ...segment("b", 4), track: 1 };
		const edit = { segments: [a, b], tracks: 2, masterVolumeDb: 0 };
		const next = applySegmentDrag(
			edit,
			dragState("move", {
				group: [
					{ ...member("a", 0), originTrack: 0 },
					{ ...member("b", 4), originTrack: 1 },
				],
				originStart: 0,
				ghostStart: 2,
				ghostTrack: 2,
				ghostStarts: [
					{ id: "a", start: 2 },
					{ id: "b", start: 6 },
				],
			}),
		);
		expect(next.tracks).toBe(4);
		expect(
			next.segments.map(({ timelineStart, track }) => ({
				timelineStart,
				track,
			})),
		).toEqual([
			{ timelineStart: 2, track: 2 },
			{ timelineStart: 6, track: 3 },
		]);
	});

	test("returns signed edge-scroll request without mutating DOM", () => {
		expect(timelineScrollRequest(pointer(100, 95))).toBe(86);
		expect(timelineScrollRequest(pointer(100, 5))).toBe(-86);
		expect(timelineScrollRequest(pointer(100, 50))).toBe(0);
	});
});

describe("clampPointToRect", () => {
	const rect = { left: 260, right: 900, top: 80, bottom: 600 };

	test("keeps a point inside the rect unchanged", () => {
		expect(clampPointToRect(500, 300, rect)).toEqual({ x: 500, y: 300 });
	});

	test("clamps a point above and left of the rect to its top-left corner", () => {
		expect(clampPointToRect(0, 0, rect)).toEqual({ x: 260, y: 80 });
	});

	test("clamps a point below and right of the rect to its bottom-right corner", () => {
		expect(clampPointToRect(2000, 2000, rect)).toEqual({ x: 900, y: 600 });
	});

	test("clamps only the out-of-bounds axis", () => {
		expect(clampPointToRect(0, 300, rect)).toEqual({ x: 260, y: 300 });
		expect(clampPointToRect(500, 0, rect)).toEqual({ x: 500, y: 80 });
		expect(clampPointToRect(2000, 300, rect)).toEqual({ x: 900, y: 300 });
		expect(clampPointToRect(500, 2000, rect)).toEqual({ x: 500, y: 600 });
	});
});

describe("marqueeOverlayOffset", () => {
	// Tracks container at (260, 48); the overlay is an absolutely positioned
	// child of the scroll container, so its CSS offsets are relative to the
	// scrolled content.
	const rect = { left: 260, top: 48 };

	test("places the box at the dragged corner when unscrolled", () => {
		const offset = marqueeOverlayOffset(700, 500, 900, 700, rect, 0, 0);
		expect(offset).toEqual({ left: 440, top: 452 });
	});

	test("anchors at the pointer when dragging up and left", () => {
		const offset = marqueeOverlayOffset(700, 500, 300, 100, rect, 0, 0);
		expect(offset).toEqual({ left: 40, top: 52 });
	});

	test("adds the scroll position back for a scrolled container", () => {
		const offset = marqueeOverlayOffset(700, 500, 900, 700, rect, 0, 300);
		expect(offset).toEqual({ left: 440, top: 752 });
		const both = marqueeOverlayOffset(700, 500, 900, 700, rect, 120, 300);
		expect(both).toEqual({ left: 560, top: 752 });
	});
});

describe("marqueeIntersectsSegment", () => {
	// Row box 600px wide, segment occupies 20%..60% of it (380..620), padded
	// 8px vertically inside the row (108..152).
	const rowRect = {
		left: 260,
		right: 860,
		top: 100,
		bottom: 160,
		width: 600,
	};
	const bounds = (left: number, right: number, top: number, bottom: number) =>
		({ left, right, top, bottom }) as const;

	test("a rectangle overlapping the segment selects it", () => {
		expect(
			marqueeIntersectsSegment(20, 60, rowRect, bounds(380, 450, 100, 160)),
		).toBe(true);
	});

	test("a rectangle fully containing the segment selects it", () => {
		expect(
			marqueeIntersectsSegment(20, 60, rowRect, bounds(200, 900, 50, 300)),
		).toBe(true);
	});

	test("a rectangle beside the segment leaves it unselected", () => {
		expect(
			marqueeIntersectsSegment(20, 60, rowRect, bounds(100, 379, 100, 160)),
		).toBe(false);
		expect(
			marqueeIntersectsSegment(20, 60, rowRect, bounds(621, 900, 100, 160)),
		).toBe(false);
	});

	test("a rectangle outside the row's vertical padding leaves it unselected", () => {
		// The top of the marquee sits in the row's 8px padding band.
		expect(
			marqueeIntersectsSegment(20, 60, rowRect, bounds(380, 450, 90, 107)),
		).toBe(false);
	});

	test("an empty row never matches", () => {
		expect(
			marqueeIntersectsSegment(
				20,
				60,
				{ ...rowRect, width: 0 },
				bounds(0, 900, 0, 300),
			),
		).toBe(false);
	});

	test("crossing the border selects; resting exactly on it does not", () => {
		expect(
			marqueeIntersectsSegment(20, 60, rowRect, bounds(619, 620, 108, 152)),
		).toBe(true);
		expect(
			marqueeIntersectsSegment(20, 60, rowRect, bounds(620, 621, 108, 152)),
		).toBe(false);
	});
});
