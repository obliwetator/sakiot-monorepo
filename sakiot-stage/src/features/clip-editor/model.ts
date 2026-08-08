export type ClipSource = "clip" | "session" | "session-silence-free";

export interface SegmentEffects {
	volumeDb: number;
	pitchCents: number;
	rate: number;
	bassDb: number;
	trebleDb: number;
}

export interface TimelineSegment {
	id: string;
	track: number;
	source: ClipSource;
	sourceId: string;
	sourceIn: number;
	sourceOut: number;
	timelineStart: number;
	effects: SegmentEffects;
}

export interface ClipEdit {
	segments: TimelineSegment[];
	tracks: number;
	masterVolumeDb: number;
}

export const MIN_SEGMENT_SECONDS = 0.05;

export const DEFAULT_EFFECTS: SegmentEffects = {
	volumeDb: 0,
	pitchCents: 0,
	rate: 1,
	bassDb: 0,
	trebleDb: 0,
};

let nextSegmentId = 0;

export function newSegmentId(): string {
	nextSegmentId += 1;
	return `seg-${nextSegmentId}`;
}

export function emptyEdit(): ClipEdit {
	return { segments: [], tracks: 1, masterVolumeDb: 0 };
}

export function makeSegment(
	source: ClipSource,
	sourceId: string,
	sourceIn: number,
	sourceOut: number,
	timelineStart: number,
	track: number,
): TimelineSegment {
	return {
		id: newSegmentId(),
		track,
		source,
		sourceId,
		sourceIn,
		sourceOut,
		timelineStart,
		effects: { ...DEFAULT_EFFECTS },
	};
}

export function segmentDuration(segment: TimelineSegment): number {
	return Math.max(0, segment.sourceOut - segment.sourceIn);
}

export function segmentEnd(segment: TimelineSegment): number {
	return segment.timelineStart + segmentDuration(segment);
}

export function editDuration(edit: ClipEdit): number {
	return edit.segments.reduce(
		(max, segment) => Math.max(max, segmentEnd(segment)),
		0,
	);
}

export function addSegment(edit: ClipEdit, segment: TimelineSegment): ClipEdit {
	return { ...edit, segments: [...edit.segments, segment] };
}

export function updateSegment(
	edit: ClipEdit,
	id: string,
	patch: Partial<TimelineSegment>,
): ClipEdit {
	return {
		...edit,
		segments: edit.segments.map((segment) =>
			segment.id === id ? { ...segment, ...patch } : segment,
		),
	};
}

export function removeSegment(edit: ClipEdit, id: string): ClipEdit {
	return { ...edit, segments: edit.segments.filter((s) => s.id !== id) };
}

export function moveSegment(
	edit: ClipEdit,
	id: string,
	timelineStart: number,
	track: number,
): ClipEdit {
	return updateSegment(edit, id, {
		timelineStart: Math.max(0, timelineStart),
		track: Math.max(0, track),
	});
}

export function setSegmentRange(
	edit: ClipEdit,
	id: string,
	sourceIn: number,
	sourceOut: number,
): ClipEdit {
	return updateSegment(edit, id, {
		sourceIn,
		sourceOut: Math.max(sourceIn + MIN_SEGMENT_SECONDS, sourceOut),
	});
}

export function splitSegment(
	edit: ClipEdit,
	id: string,
	atSec: number,
): ClipEdit {
	const segment = edit.segments.find((s) => s.id === id);
	if (!segment) return edit;
	const start = segment.timelineStart;
	const end = segmentEnd(segment);
	if (
		atSec - start < MIN_SEGMENT_SECONDS ||
		end - atSec < MIN_SEGMENT_SECONDS
	) {
		return edit;
	}
	const second: TimelineSegment = {
		...segment,
		id: newSegmentId(),
		sourceIn: segment.sourceIn + (atSec - start),
		timelineStart: atSec,
	};
	return { ...edit, segments: [...edit.segments, second] };
}

export function addTrack(edit: ClipEdit): ClipEdit {
	return { ...edit, tracks: edit.tracks + 1 };
}

/**
 * Hard anti-overlap snap: a clip of `duration` starting at `ghostStart` on
 * `track` may never overlap a neighbour. If it would, it is placed against
 * that neighbour's edge - ending where the neighbour starts when the cursor
 * is over the neighbour's left half, starting where the neighbour ends when
 * it is over the right half. The neighbour under the cursor wins when the
 * ghost overlaps several.
 *
 * A single snap can land on a second neighbour (clips sitting back to back),
 * so the resolution cascades: each candidate side is checked for further
 * overlaps and the recursion continues until a stable, overlap-free position
 * is found. When neither side is free, the side with fewer overlaps wins and
 * the cursor decides ties; the depth cap guarantees termination.
 */
export function snapToNeighbors(
	ghostStart: number,
	duration: number,
	segments: readonly TimelineSegment[],
	excludeId: string,
	track: number,
	rawStart: number,
): number {
	return resolveOverlap(
		ghostStart,
		duration,
		segments,
		excludeId,
		track,
		rawStart,
		segments.length + 2,
	);
}

function overlapsAny(
	start: number,
	duration: number,
	segments: readonly TimelineSegment[],
	excludeId: string,
	track: number,
): boolean {
	const end = start + duration;
	return segments.some(
		(segment) =>
			segment.track === track &&
			segment.id !== excludeId &&
			segment.timelineStart < end &&
			segmentEnd(segment) > start,
	);
}

function resolveOverlap(
	start: number,
	duration: number,
	segments: readonly TimelineSegment[],
	excludeId: string,
	track: number,
	rawStart: number,
	depth: number,
): number {
	const end = start + duration;
	const overlapping = segments.filter(
		(segment) =>
			segment.track === track &&
			segment.id !== excludeId &&
			segment.timelineStart < end &&
			segmentEnd(segment) > start,
	);
	if (overlapping.length === 0) return start;
	const cursorMid = rawStart + duration / 2;
	const primary = overlapping.reduce((best, segment) => {
		const bestMid = (best.timelineStart + segmentEnd(best)) / 2;
		const mid = (segment.timelineStart + segmentEnd(segment)) / 2;
		return Math.abs(cursorMid - mid) < Math.abs(cursorMid - bestMid)
			? segment
			: best;
	});
	const mid = (primary.timelineStart + segmentEnd(primary)) / 2;
	const leftCand = Math.max(0, primary.timelineStart - duration);
	const rightCand = segmentEnd(primary);
	if (leftCand === start && rightCand === start) return start;
	const leftFree = !overlapsAny(leftCand, duration, segments, excludeId, track);
	const rightFree = !overlapsAny(
		rightCand,
		duration,
		segments,
		excludeId,
		track,
	);
	const candidate =
		leftFree !== rightFree
			? leftFree
				? leftCand
				: rightCand
			: cursorMid < mid
				? leftCand
				: rightCand;
	if (candidate === start || depth <= 0) return candidate;
	if (!overlapsAny(candidate, duration, segments, excludeId, track)) {
		return candidate;
	}
	return resolveOverlap(
		candidate,
		duration,
		segments,
		excludeId,
		track,
		rawStart,
		depth - 1,
	);
}
