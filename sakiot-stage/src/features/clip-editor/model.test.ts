import { describe, expect, test } from "bun:test";
import {
	DEFAULT_EFFECTS,
	effectiveRate,
	expandMergeGroups,
	leftEdgeFloor,
	mergeBlockReason,
	mergeSegments,
	resizeSelectedSegments,
	rightEdgeCeiling,
	segmentDuration,
	segmentEnd,
	setSegmentPitch,
	setSegmentSpeed,
	snapToNeighbors,
	sourcePositionAt,
	splitSegment,
	type TimelineSegment,
	unmergeSegments,
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

describe("resizeSelectedSegments", () => {
	const slower = (ids: string[]) => (edit: ClipEdit) =>
		resizeSelectedSegments(edit, ids, (_id, effects) => ({
			...effects,
			rate: 0.5,
		}));
	const faster = (ids: string[]) => (edit: ClipEdit) =>
		resizeSelectedSegments(edit, ids, (_id, effects) => ({
			...effects,
			rate: 2,
		}));

	test("a contraction pulls its snapped selected follower", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 4, 4);
		const next = faster(["a", "b"])(editWith(a, b));
		expect(next.segments[0]?.timelineStart).toBe(0);
		expect(segmentDuration(next.segments[0] as TimelineSegment)).toBe(2);
		expect(next.segments[1]?.timelineStart).toBe(2);
		expect(segmentDuration(next.segments[1] as TimelineSegment)).toBe(2);
	});

	test("an extension carries its snapped selected follower", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 4, 4);
		const next = slower(["a", "b"])(editWith(a, b));
		expect(segmentDuration(next.segments[0] as TimelineSegment)).toBe(8);
		expect(next.segments[1]?.timelineStart).toBe(8);
		expect(segmentDuration(next.segments[1] as TimelineSegment)).toBe(8);
	});

	test("an unselected snapped neighbour stays put", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 4, 4);
		const c = seg("c", 0, 8, 4);
		const next = faster(["a", "b"])(editWith(a, b, c));
		// A and B contract together; the unselected C is never pulled.
		expect(next.segments[0]?.timelineStart).toBe(0);
		expect(next.segments[1]?.timelineStart).toBe(2);
		expect(next.segments[2]?.timelineStart).toBe(8);
	});

	test("a growth reaches an unselected neighbour and pushes it", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 6, 4);
		const next = slower(["a"])(editWith(a, b));
		expect(segmentDuration(next.segments[0] as TimelineSegment)).toBe(8);
		expect(next.segments[1]?.timelineStart).toBe(8);
	});

	test("a gap between selected segments stays a gap", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 6, 4);
		const next = faster(["a", "b"])(editWith(a, b));
		expect(segmentDuration(next.segments[0] as TimelineSegment)).toBe(2);
		// Not snapped: B is not pulled towards A.
		expect(next.segments[1]?.timelineStart).toBe(6);
	});

	test("non-resizing effect changes leave positions alone", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 4, 4);
		const next = resizeSelectedSegments(
			editWith(a, b),
			["a", "b"],
			(_id, effects) => ({
				...effects,
				volumeDb: -6,
			}),
		);
		expect(next.segments[0]?.timelineStart).toBe(0);
		expect(next.segments[1]?.timelineStart).toBe(4);
		expect(next.segments[0]?.effects.volumeDb).toBe(-6);
		expect(next.segments[1]?.effects.volumeDb).toBe(-6);
	});
});

describe("effectiveRate", () => {
	test("pitch does not change source consumption or duration", () => {
		const a = seg("a", 0, 0, 4);
		a.effects.rate = 2;
		expect(effectiveRate(a.effects)).toBe(2);
		a.effects.pitchCents = 1200;
		expect(effectiveRate(a.effects)).toBe(2);
		a.effects.pitchCents = -1200;
		expect(effectiveRate(a.effects)).toBe(2);
	});
});

describe("effect tails", () => {
	test("adds a fixed post-content extent that speed does not scale", () => {
		const a = seg("a", 0, 0, 4);
		a.effects.rate = 2;
		a.effects.tailSeconds = 2;
		expect(segmentDuration(a)).toBe(4);
	});

	test("resizing a tail carries a snapped selected follower", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 4, 4);
		const next = resizeSelectedSegments(
			editWith(a, b),
			["a", "b"],
			(_id, effects) => ({ ...effects, tailSeconds: 2 }),
		);
		expect(segmentDuration(next.segments[0] as TimelineSegment)).toBe(6);
		expect(next.segments[1]?.timelineStart).toBe(6);
	});
});

describe("setSegmentPitch", () => {
	test("pitch up preserves the segment and neighbour positions", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 4, 4);
		const next = setSegmentPitch(editWith(a, b), "a", 1200);
		expect(next.segments[0]?.effects.pitchCents).toBe(1200);
		expect(segmentDuration(next.segments[0] as TimelineSegment)).toBe(4);
		expect(next.segments[1]?.timelineStart).toBe(4);
	});

	test("pitch down also preserves duration", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 4, 4);
		const next = setSegmentPitch(editWith(a, b), "a", -1200);
		expect(segmentDuration(next.segments[0] as TimelineSegment)).toBe(4);
		expect(next.segments[1]?.timelineStart).toBe(4);
	});

	test("pitch keeps an existing speed extent", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 4, 4);
		const speed = setSegmentSpeed(editWith(a, b), "a", 2); // box 0..2
		const pitched = setSegmentPitch(speed, "a", -1200);
		expect(segmentDuration(pitched.segments[0] as TimelineSegment)).toBe(2);
		expect(pitched.segments[1]?.timelineStart).toBe(4);
	});

	test("a bad pitch is rejected", () => {
		const a = seg("a", 0, 0, 4);
		const edit = editWith(a);
		expect(setSegmentPitch(edit, "a", Number.NaN)).toBe(edit);
	});
});

describe("rightEdgeCeiling", () => {
	test("a clip snapped to a shrunken edge bounds the extension", () => {
		const a = seg("a", 0, 0, 10);
		a.sourceOut = 6; // right edge shrunk to 6
		const b = seg("b", 0, 6, 4); // snapped onto the shrunken edge
		expect(rightEdgeCeiling([a, b], 0, "a", 0)).toBe(6);
	});

	test("a neighbour with a gap does not bound the extension", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 10, 4);
		expect(rightEdgeCeiling([a, b], 0, "a", 0)).toBe(10);
	});

	test("returns null when no neighbour starts at or after the box", () => {
		const a = seg("a", 0, 0, 4);
		expect(rightEdgeCeiling([a], 0, "a", 0)).toBeNull();
	});

	test("neighbours on other tracks never bound the extension", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 1, 2, 4);
		expect(rightEdgeCeiling([a, b], 0, "a", 0)).toBeNull();
	});

	test("the ceiling follows the dragged segment's effective rate", () => {
		const a = seg("a", 0, 0, 10);
		a.sourceOut = 6;
		a.effects.rate = 2; // box [0, 3] before the shrink, shrink keeps end at 3
		const b = seg("b", 0, 3, 4); // snapped onto the shrunken edge
		expect(rightEdgeCeiling([a, b], 0, "a", 0)).toBe(3);
	});
});

describe("leftEdgeFloor", () => {
	test("a clip snapped to a shrunken edge bounds the extension", () => {
		const a = seg("a", 0, 0, 4);
		const b = seg("b", 0, 4, 10);
		b.sourceIn = 4; // left edge shrunk to 4, box [4, 10]
		expect(leftEdgeFloor([a, b], 0, "b", 10)).toBe(4);
	});

	test("returns zero when no neighbour ends at or before the box", () => {
		const a = seg("a", 0, 0, 4);
		expect(leftEdgeFloor([a], 0, "a", 4)).toBe(0);
	});

	test("neighbours on other tracks never bound the extension", () => {
		const a = seg("a", 0, 0, 10);
		const b = seg("b", 1, 0, 4); // would bound the box if track were ignored
		expect(leftEdgeFloor([a, b], 0, "a", 4)).toBe(0);
	});

	test("the floor follows the dragged segment's effective rate", () => {
		const a = seg("a", 0, 0, 2);
		const b = seg("b", 0, 2, 10);
		b.sourceIn = 2;
		b.effects.rate = 2; // box [2, 6]
		expect(leftEdgeFloor([a, b], 0, "b", 6)).toBe(2);
	});
});

describe("splitSegment", () => {
	test("splits into a left and a right clip around the playhead", () => {
		const a = seg("a", 0, 0, 10);
		const next = splitSegment(editWith(a), "a", 4);
		expect(next.segments).toHaveLength(2);
		const left = next.segments[0] as TimelineSegment;
		const right = next.segments[1] as TimelineSegment;
		expect(left.sourceIn).toBe(0);
		expect(left.sourceOut).toBe(4);
		expect(segmentDuration(left)).toBe(4);
		expect(right.sourceIn).toBe(4);
		expect(right.sourceOut).toBe(10);
		expect(right.timelineStart).toBe(4);
		expect(segmentDuration(right)).toBe(6);
		expect(segmentEnd(left)).toBe(4);
		expect(right.timelineStart).toBe(segmentEnd(left));
	});

	test("the split point follows the effective rate", () => {
		const a = seg("a", 0, 0, 10);
		a.effects.rate = 2; // box [0, 5]
		const next = splitSegment(editWith(a), "a", 3);
		const left = next.segments[0] as TimelineSegment;
		const right = next.segments[1] as TimelineSegment;
		expect(left.sourceOut).toBe(6);
		expect(segmentDuration(left)).toBe(3);
		expect(right.sourceIn).toBe(6);
		expect(right.timelineStart).toBe(3);
	});

	test("a source-offset segment keeps its offset in both halves", () => {
		const a = seg("a", 0, 0, 10);
		a.sourceIn = 2;
		a.sourceOut = 8; // box [0, 6]
		const next = splitSegment(editWith(a), "a", 2);
		const left = next.segments[0] as TimelineSegment;
		const right = next.segments[1] as TimelineSegment;
		expect(left.sourceIn).toBe(2);
		expect(left.sourceOut).toBe(4);
		expect(right.sourceIn).toBe(4);
		expect(right.timelineStart).toBe(2);
	});

	test("a split too close to either edge is rejected", () => {
		const a = seg("a", 0, 0, 10);
		expect(splitSegment(editWith(a), "a", 0.01).segments).toHaveLength(1);
		expect(splitSegment(editWith(a), "a", 9.99).segments).toHaveLength(1);
	});

	test("a reversed segment splits at the mirrored source position", () => {
		const a = seg("a", 0, 0, 10);
		a.effects.reverse = true;
		const next = splitSegment(editWith(a), "a", 4);
		const left = next.segments[0] as TimelineSegment;
		const right = next.segments[1] as TimelineSegment;
		// The box plays [0, 10] backwards, so 4s in is at source second 6: the
		// left half covers [6, 10] (still reversed) and the right [0, 6].
		expect(left.sourceIn).toBe(6);
		expect(left.sourceOut).toBe(10);
		expect(left.effects.reverse).toBe(true);
		expect(segmentDuration(left)).toBe(4);
		expect(right.sourceIn).toBe(0);
		expect(right.sourceOut).toBe(6);
		expect(right.effects.reverse).toBe(true);
		expect(right.timelineStart).toBe(4);
		expect(segmentDuration(right)).toBe(6);
	});

	test("keeps one configured tail on the final timeline piece", () => {
		const a = seg("a", 0, 0, 10);
		a.effects.tailSeconds = 2;
		const next = splitSegment(editWith(a), "a", 4);
		const left = next.segments[0] as TimelineSegment;
		const right = next.segments[1] as TimelineSegment;
		expect(left.effects.tailSeconds).toBe(0);
		expect(segmentDuration(left)).toBe(4);
		expect(right.effects.tailSeconds).toBe(2);
		expect(segmentEnd(right)).toBe(12);
	});
});

describe("sourcePositionAt", () => {
	test("walks forward from source-in at the effective rate", () => {
		const a = seg("a", 0, 0, 10);
		a.effects.rate = 2;
		expect(sourcePositionAt(a, 3)).toBe(6);
	});

	test("walks backward from source-out when reversed", () => {
		const a = seg("a", 0, 0, 10);
		a.effects.reverse = true;
		expect(sourcePositionAt(a, 4)).toBe(6);
	});

	test("clamps before the segment start", () => {
		const a = seg("a", 0, 2, 10);
		a.sourceIn = 4;
		expect(sourcePositionAt(a, -5)).toBe(4);
	});

	test("clamps to the source boundary throughout the effect tail", () => {
		const forward = seg("forward", 0, 0, 4);
		forward.effects.tailSeconds = 2;
		expect(sourcePositionAt(forward, 5)).toBe(4);
		const reverse = seg("reverse", 0, 0, 4);
		reverse.effects.reverse = true;
		reverse.effects.tailSeconds = 2;
		expect(sourcePositionAt(reverse, 5)).toBe(0);
	});
});

/** A segment trimmed out of the shared source clip `sourceId`. */
function piece(
	id: string,
	sourceId: string,
	sourceIn: number,
	sourceOut: number,
	timelineStart: number,
	track = 0,
) {
	const segment = seg(id, track, timelineStart, sourceOut - sourceIn);
	segment.sourceId = sourceId;
	segment.sourceIn = sourceIn;
	segment.sourceOut = sourceOut;
	return segment;
}

describe("mergeBlockReason", () => {
	test("a snapped forward chain is mergeable", () => {
		const a = piece("a", "src", 0, 4, 0);
		const b = piece("b", "src", 4, 10, 4);
		expect(mergeBlockReason([a, b])).toBeNull();
	});

	test("a three-piece chain merges when every border snaps", () => {
		const a = piece("a", "src", 0, 4, 0);
		const b = piece("b", "src", 4, 7, 4);
		const c = piece("c", "src", 7, 10, 7);
		expect(mergeBlockReason([a, b, c])).toBeNull();
	});

	test("a reversed chain snapped on the timeline is mergeable", () => {
		const a = piece("a", "src", 6, 10, 0);
		a.effects.reverse = true;
		const b = piece("b", "src", 0, 6, 4);
		b.effects.reverse = true;
		expect(mergeBlockReason([a, b])).toBeNull();
	});

	test("repeating the same source window is mergeable", () => {
		const a = piece("a", "src", 0, 4, 0);
		const b = piece("b", "src", 0, 4, 4);
		expect(mergeBlockReason([a, b])).toBeNull();
	});

	test("snapped clips from different sources are mergeable", () => {
		const a = piece("a", "src1", 0, 4, 0);
		const b = piece("b", "src2", 4, 8, 4);
		expect(mergeBlockReason([a, b])).toBeNull();
	});

	test("snapped clips with different effects are mergeable", () => {
		const a = piece("a", "src", 0, 4, 0);
		const b = piece("b", "src", 4, 8, 4);
		b.effects.volumeDb = -6;
		expect(mergeBlockReason([a, b])).toBeNull();
	});

	test("a single segment is refused", () => {
		expect(mergeBlockReason([piece("a", "src", 0, 4, 0)])).toBe("too-few");
	});

	test("a selection across tracks is refused", () => {
		const a = piece("a", "src", 0, 4, 0);
		const b = piece("b", "src", 4, 8, 4, 1);
		expect(mergeBlockReason([a, b])).toBe("multi-track");
	});

	test("a gap on the timeline is refused", () => {
		const a = piece("a", "src", 0, 4, 0);
		const b = piece("b", "src", 4, 8, 6);
		expect(mergeBlockReason([a, b])).toBe("not-snapped");
	});

	test("an overlap is refused", () => {
		const a = piece("a", "src", 0, 4, 0);
		const b = piece("b", "src", 4, 8, 3);
		expect(mergeBlockReason([a, b])).toBe("not-snapped");
	});
});

describe("mergeSegments", () => {
	test("tags the chain with one group id and keeps every segment", () => {
		const a = piece("a", "src", 0, 4, 0);
		const b = piece("b", "src", 4, 10, 4);
		const result = mergeSegments(editWith(a, b), ["a", "b"]);
		expect(result).not.toBeNull();
		const { edit, groupId } = result as {
			edit: ReturnType<typeof editWith>;
			groupId: string;
		};
		expect(groupId.startsWith("group-")).toBe(true);
		expect(edit.segments).toHaveLength(2);
		expect(edit.segments[0]?.mergeGroup).toBe(groupId);
		expect(edit.segments[1]?.mergeGroup).toBe(groupId);
		expect(edit.segments[0]?.sourceId).toBe("src");
		expect(edit.segments[1]?.sourceId).toBe("src");
	});

	test("repeated windows merge without losing audio", () => {
		const a = piece("a", "src", 0, 4, 0);
		const b = piece("b", "src", 0, 4, 4);
		const result = mergeSegments(editWith(a, b), ["a", "b"]);
		expect(result).not.toBeNull();
		const segments = (result as { edit: { segments: TimelineSegment[] } }).edit
			.segments as TimelineSegment[];
		expect(segments).toHaveLength(2);
		expect(segments[0]?.mergeGroup).toBe(segments[1]?.mergeGroup);
		expect(segmentDuration(segments[0] as TimelineSegment)).toBe(4);
		expect(segmentDuration(segments[1] as TimelineSegment)).toBe(4);
	});

	test("snapped clips from different sources merge", () => {
		const a = piece("a", "src1", 0, 4, 0);
		const b = piece("b", "src2", 4, 8, 4);
		const result = mergeSegments(editWith(a, b), ["a", "b"]);
		expect(result).not.toBeNull();
		const segments = (result as { edit: { segments: TimelineSegment[] } }).edit
			.segments as TimelineSegment[];
		expect(segments[0]?.sourceId).toBe("src1");
		expect(segments[1]?.sourceId).toBe("src2");
		expect(segments[0]?.mergeGroup).toBe(segments[1]?.mergeGroup);
	});

	test("merging two existing groups unifies them", () => {
		const a = piece("a", "src", 0, 4, 0);
		a.mergeGroup = "group-old-1";
		const b = piece("b", "src", 4, 8, 4);
		b.mergeGroup = "group-old-2";
		const result = mergeSegments(editWith(a, b), ["a", "b"]);
		expect(result).not.toBeNull();
		const { groupId } = result as { groupId: string };
		expect(groupId).not.toBe("group-old-1");
		expect(groupId).not.toBe("group-old-2");
		const segments = (result as { edit: { segments: TimelineSegment[] } }).edit
			.segments as TimelineSegment[];
		expect(segments[0]?.mergeGroup).toBe(groupId);
		expect(segments[1]?.mergeGroup).toBe(groupId);
	});

	test("unselected segments are untouched", () => {
		const a = piece("a", "src", 0, 4, 0);
		const b = piece("b", "src", 4, 8, 4);
		const c = piece("c", "src2", 0, 4, 8, 1);
		const result = mergeSegments(editWith(a, b, c), ["a", "b"]);
		expect(result?.edit.segments).toHaveLength(3);
		expect(result?.edit.segments[2]?.mergeGroup).toBeUndefined();
	});

	test("merges ids given in any order", () => {
		const a = piece("a", "src", 0, 4, 0);
		const b = piece("b", "src", 4, 8, 4);
		const result = mergeSegments(editWith(a, b), ["b", "a"]);
		expect(result?.edit.segments[0]?.mergeGroup).toBe(
			result?.edit.segments[1]?.mergeGroup,
		);
	});

	test("returns null when the merge is refused", () => {
		const a = piece("a", "src", 0, 4, 0);
		const b = piece("b", "src", 4, 8, 6);
		expect(mergeSegments(editWith(a, b), ["a", "b"])).toBeNull();
	});
});

describe("unmergeSegments", () => {
	test("strips the group id from the given segments only", () => {
		const a = piece("a", "src", 0, 4, 0);
		a.mergeGroup = "group-1";
		const b = piece("b", "src", 4, 8, 4);
		b.mergeGroup = "group-1";
		const c = piece("c", "src2", 0, 4, 8, 1);
		c.mergeGroup = "group-2";
		const edit = unmergeSegments(editWith(a, b, c), ["a", "b"]);
		expect(edit.segments[0]?.mergeGroup).toBeUndefined();
		expect(edit.segments[1]?.mergeGroup).toBeUndefined();
		expect(edit.segments[2]?.mergeGroup).toBe("group-2");
	});

	test("leaves ungrouped segments alone", () => {
		const a = piece("a", "src", 0, 4, 0);
		const edit = unmergeSegments(editWith(a), ["a"]);
		expect(edit.segments[0]?.mergeGroup).toBeUndefined();
	});
});

describe("expandMergeGroups", () => {
	test("lone ids pass through unchanged", () => {
		const a = piece("a", "src", 0, 4, 0);
		expect(expandMergeGroups([a], ["a"])).toEqual(["a"]);
	});

	test("selecting one member selects every member", () => {
		const a = piece("a", "src", 0, 4, 0);
		a.mergeGroup = "group-1";
		const b = piece("b", "src", 4, 8, 4);
		b.mergeGroup = "group-1";
		expect(expandMergeGroups([a, b], ["a"])).toEqual(["a", "b"]);
	});

	test("mixing members and lone segments keeps both", () => {
		const a = piece("a", "src", 0, 4, 0);
		a.mergeGroup = "group-1";
		const b = piece("b", "src", 4, 8, 4);
		b.mergeGroup = "group-1";
		const c = piece("c", "src2", 0, 4, 8, 1);
		expect(expandMergeGroups([a, b, c], ["b", "c"])).toEqual(["a", "b", "c"]);
	});

	test("unknown ids are dropped", () => {
		const a = piece("a", "src", 0, 4, 0);
		expect(expandMergeGroups([a], ["nope"])).toEqual([]);
	});
});
