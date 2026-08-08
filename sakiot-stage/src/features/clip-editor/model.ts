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

/**
 * Pitch shift as a rate multiplier: 1200 cents is one octave, matching the
 * Web Audio detune semantics that fold into the computed playback rate.
 */
export function pitchFactor(effects: SegmentEffects): number {
	return 2 ** (effects.pitchCents / 1200);
}

/**
 * Effective playback rate of a segment: the Web Audio engine consumes the
 * source buffer at `playbackRate * 2^(detune/1200)` per real second, so both
 * speed and pitch resize the audible extent of a segment on the timeline.
 */
export function effectiveRate(effects: SegmentEffects): number {
	return effects.rate * pitchFactor(effects);
}

/**
 * Real-time duration of the segment on the timeline: the trimmed source
 * content played back at the segment's effective speed and pitch.
 */
export function segmentDuration(segment: TimelineSegment): number {
	return (
		Math.max(0, segment.sourceOut - segment.sourceIn) /
		Math.max(0.01, effectiveRate(segment.effects))
	);
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
	const splitSource =
		segment.sourceIn + (atSec - start) * effectiveRate(segment.effects);
	const first: TimelineSegment = {
		...segment,
		sourceOut: splitSource,
	};
	const second: TimelineSegment = {
		...segment,
		id: newSegmentId(),
		sourceIn: splitSource,
		timelineStart: atSec,
	};
	return {
		...edit,
		segments: [...edit.segments.map((s) => (s.id === id ? first : s)), second],
	};
}

export function addTrack(edit: ClipEdit): ClipEdit {
	return { ...edit, tracks: edit.tracks + 1 };
}

/**
 * Sets a segment's playback speed and resizes its timeline extent to match:
 * the box becomes `content / effectiveRate` wide, so faster clips shrink and
 * slower clips grow. An extension pushes every follower it reaches on the
 * same track right (staying snapped to the chain); a contraction never pulls
 * followers.
 */
export function setSegmentSpeed(
	edit: ClipEdit,
	id: string,
	rate: number,
): ClipEdit {
	if (!Number.isFinite(rate) || rate <= 0) return edit;
	const segment = edit.segments.find((s) => s.id === id);
	if (!segment || segment.effects.rate === rate) return edit;
	const oldEnd = segmentEnd(segment);
	const updated = updateSegment(edit, id, {
		effects: { ...segment.effects, rate },
	});
	return resizeSegmentTo(updated, id, oldEnd);
}

/**
 * Sets a segment's pitch and resizes its timeline extent to match, exactly
 * like the speed control: the pitch shift changes how fast the source buffer
 * is consumed, so the box follows the audible duration.
 */
export function setSegmentPitch(
	edit: ClipEdit,
	id: string,
	pitchCents: number,
): ClipEdit {
	if (!Number.isFinite(pitchCents)) return edit;
	const segment = edit.segments.find((s) => s.id === id);
	if (!segment || segment.effects.pitchCents === pitchCents) return edit;
	const oldEnd = segmentEnd(segment);
	const updated = updateSegment(edit, id, {
		effects: { ...segment.effects, pitchCents },
	});
	return resizeSegmentTo(updated, id, oldEnd);
}

function resizeSegmentTo(edit: ClipEdit, id: string, oldEnd: number): ClipEdit {
	const segment = edit.segments.find((s) => s.id === id);
	if (!segment) return edit;
	const content = segment.sourceOut - segment.sourceIn;
	const newEnd =
		segment.timelineStart + content / effectiveRate(segment.effects);
	if (newEnd <= oldEnd) return edit;
	const followers = edit.segments
		.filter(
			(follower) =>
				follower.track === segment.track &&
				follower.id !== id &&
				follower.timelineStart >= oldEnd,
		)
		.sort((a, b) => a.timelineStart - b.timelineStart);
	const shifts = new Map<string, number>();
	let boundary = newEnd;
	for (const follower of followers) {
		if (follower.timelineStart >= boundary) break;
		shifts.set(follower.id, boundary);
		boundary += segmentEnd(follower) - follower.timelineStart;
	}
	if (shifts.size === 0) return edit;
	return {
		...edit,
		segments: edit.segments.map((follower) =>
			shifts.has(follower.id)
				? {
						...follower,
						timelineStart: shifts.get(follower.id) ?? follower.timelineStart,
					}
				: follower,
		),
	};
}

/**
 * Hard anti-overlap snap: a clip of `duration` starting at `ghostStart` on
 * `track` may never overlap a neighbour. If it would, it is moved to the
 * nearest valid start position - a start from which the clip overlaps no
 * neighbour on the track - choosing the position closest to the cursor.
 *
 * Valid starts are the complement of the forbidden start intervals, one per
 * neighbour: a start `s` overlaps a neighbour [a, b] exactly when
 * `s > a - duration` and `s < b`, so the forbidden interval is
 * (a - duration, b). Merging those intervals and scanning their edges finds
 * every adjacent valid position; the one nearest the cursor wins, so the
 * cursor side still decides which neighbour edge to snap against.
 */
export function snapToNeighbors(
	ghostStart: number,
	duration: number,
	segments: readonly TimelineSegment[],
	excludeId: string,
	track: number,
	rawStart: number,
): number {
	if (!overlapsAny(ghostStart, duration, segments, excludeId, track)) {
		return ghostStart;
	}
	const forbidden = segments
		.filter((segment) => segment.track === track && segment.id !== excludeId)
		.map((segment) => ({
			start: segment.timelineStart - duration,
			end: segmentEnd(segment),
		}))
		.sort((a, b) => a.start - b.start);
	const merged: Array<{ start: number; end: number }> = [];
	for (const interval of forbidden) {
		const last = merged[merged.length - 1];
		if (last && interval.start < last.end) {
			last.end = Math.max(last.end, interval.end);
		} else {
			merged.push({ ...interval });
		}
	}
	const candidates: number[] = [];
	if (merged[0] && merged[0].start > 0) candidates.push(0);
	for (const interval of merged) {
		if (interval.start > 0) candidates.push(interval.start);
		candidates.push(interval.end);
	}
	let best = candidates[0] ?? 0;
	for (const candidate of candidates) {
		if (Math.abs(candidate - rawStart) < Math.abs(best - rawStart)) {
			best = candidate;
		}
	}
	return Math.max(0, best);
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

/**
 * Nearest same-track neighbour end that bounds a left-edge trim extension:
 * the box start may not move left of it, so a clip snapped against the
 * shrunken edge is never overlapped when the edge grows back. The box end is
 * fixed during a left-edge drag, so only neighbours ending at or before it
 * constrain the start.
 */
export function leftEdgeFloor(
	segments: readonly TimelineSegment[],
	track: number,
	excludeId: string,
	boxEnd: number,
): number {
	let floor = 0;
	for (const neighbor of segments) {
		if (neighbor.track !== track || neighbor.id === excludeId) continue;
		const end = segmentEnd(neighbor);
		if (end <= boxEnd && end > floor) floor = end;
	}
	return floor;
}

/**
 * Nearest same-track neighbour start that bounds a right-edge trim
 * extension: the box end may not move right of it. Null when no neighbour
 * constrains the extension.
 */
export function rightEdgeCeiling(
	segments: readonly TimelineSegment[],
	track: number,
	excludeId: string,
	boxStart: number,
): number | null {
	let ceiling: number | null = null;
	for (const neighbor of segments) {
		if (neighbor.track !== track || neighbor.id === excludeId) continue;
		if (neighbor.timelineStart >= boxStart) {
			ceiling =
				ceiling === null
					? neighbor.timelineStart
					: Math.min(ceiling, neighbor.timelineStart);
		}
	}
	return ceiling;
}
