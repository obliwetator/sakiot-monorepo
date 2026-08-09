export type ClipSource = "clip" | "session" | "session-silence-free";

export interface SegmentEffects {
	volumeDb: number;
	pitchCents: number;
	rate: number;
	bassDb: number;
	midDb: number;
	trebleDb: number;
	/** Plays the trimmed content backwards; the timeline extent is unchanged. */
	reverse: boolean;
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
	/**
	 * Id of the merged unit this segment belongs to. Members render as one
	 * box on the timeline and are selected, moved and edited as a whole,
	 * while playback and export keep the individual segments, so merging
	 * never changes the audio - repeated source windows and even different
	 * source clips can be merged.
	 */
	mergeGroup?: string;
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
	midDb: 0,
	trebleDb: 0,
	reverse: false,
};

let nextSegmentId = 0;

export function newSegmentId(): string {
	nextSegmentId += 1;
	return `seg-${nextSegmentId}`;
}

let nextGroupId = 0;

export function newGroupId(): string {
	nextGroupId += 1;
	return `group-${nextGroupId}`;
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
 * Source-buffer consumption rate of a segment. Pitch shifting preserves
 * duration, so only the speed control changes the timeline extent.
 */
export function effectiveRate(effects: SegmentEffects): number {
	return effects.rate;
}

/**
 * Real-time duration of the segment on the timeline: the trimmed source
 * content played back at the selected speed. Pitch is duration-preserving.
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

/**
 * Source-buffer position audible at a timeline position: the trimmed content
 * is consumed at the effective rate, walking backwards from sourceOut when
 * the segment is reversed.
 */
export function sourcePositionAt(
	segment: TimelineSegment,
	timelineSec: number,
): number {
	const elapsed = Math.max(0, timelineSec - segment.timelineStart);
	return segment.effects.reverse
		? segment.sourceOut - elapsed * effectiveRate(segment.effects)
		: segment.sourceIn + elapsed * effectiveRate(segment.effects);
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
	const splitSource = sourcePositionAt(segment, atSec);
	// A reversed segment plays its window backwards, so the left half covers
	// the source range after the split point and the right half the range
	// before it; the direction flag carries into both halves unchanged.
	const first: TimelineSegment = segment.effects.reverse
		? { ...segment, sourceIn: splitSource }
		: { ...segment, sourceOut: splitSource };
	const second: TimelineSegment = segment.effects.reverse
		? {
				...segment,
				id: newSegmentId(),
				sourceOut: splitSource,
				timelineStart: atSec,
			}
		: {
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

export type MergeBlockReason = "too-few" | "multi-track" | "not-snapped";

export const MERGE_BLOCK_MESSAGES: Record<MergeBlockReason, string> = {
	"too-few": "Select at least two clips to merge.",
	"multi-track":
		"This operation is impossible: the selection spans multiple tracks.",
	"not-snapped":
		"This operation is illegal: the selected clips are not snapped together.",
};

/**
 * Whether the segments can be merged into one unit: at least two, all on the
 * same track, snapped end-to-end (each box starts exactly where the previous
 * one ends; gaps and overlaps are refused). Returns null when mergeable.
 */
export function mergeBlockReason(
	segments: readonly TimelineSegment[],
): MergeBlockReason | null {
	if (segments.length < 2) return "too-few";
	const tracks = new Set(segments.map((segment) => segment.track));
	if (tracks.size > 1) return "multi-track";
	const ordered = [...segments].sort(
		(a, b) => a.timelineStart - b.timelineStart,
	);
	for (let i = 1; i < ordered.length; i += 1) {
		const previous = ordered[i - 1];
		const current = ordered[i];
		if (!previous || !current) continue;
		if (
			Math.abs(current.timelineStart - segmentEnd(previous)) >= SNAP_EPSILON
		) {
			return "not-snapped";
		}
	}
	return null;
}

/**
 * Merges the selected segments into one unit by tagging them with a shared
 * group id. The segments themselves are untouched, so playback and export
 * render exactly what the chain played - repeated source windows and mixed
 * source clips merge without losing audio. Returns null when the merge
 * would be refused by `mergeBlockReason`.
 */
export function mergeSegments(
	edit: ClipEdit,
	ids: readonly string[],
): { edit: ClipEdit; groupId: string } | null {
	const selected = ids
		.map((id) => edit.segments.find((segment) => segment.id === id))
		.filter((segment): segment is TimelineSegment => Boolean(segment));
	if (mergeBlockReason(selected) !== null) return null;
	const groupId = newGroupId();
	return {
		edit: {
			...edit,
			segments: edit.segments.map((segment) =>
				ids.includes(segment.id)
					? { ...segment, mergeGroup: groupId }
					: segment,
			),
		},
		groupId,
	};
}

/** Breaks the given segments out of their merged unit (no-op without groups). */
export function unmergeSegments(
	edit: ClipEdit,
	ids: readonly string[],
): ClipEdit {
	const selected = new Set(ids);
	return {
		...edit,
		segments: edit.segments.map((segment) =>
			selected.has(segment.id)
				? { ...segment, mergeGroup: undefined }
				: segment,
		),
	};
}

/**
 * Expands ids to their merged units: selecting one member selects every
 * member, so a unit always acts as one element. Ids without a group pass
 * through unchanged.
 */
export function expandMergeGroups(
	segments: readonly TimelineSegment[],
	ids: readonly string[],
): string[] {
	const membersByGroup = new Map<string, string[]>();
	for (const segment of segments) {
		if (!segment.mergeGroup) continue;
		const members = membersByGroup.get(segment.mergeGroup) ?? [];
		members.push(segment.id);
		membersByGroup.set(segment.mergeGroup, members);
	}
	const expanded = new Set<string>();
	for (const id of ids) {
		const segment = segments.find((s) => s.id === id);
		if (!segment) continue;
		if (!segment.mergeGroup) {
			expanded.add(id);
			continue;
		}
		for (const member of membersByGroup.get(segment.mergeGroup) ?? []) {
			expanded.add(member);
		}
	}
	return [...expanded];
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

/** Sets a segment's pitch without changing its timeline extent. */
export function setSegmentPitch(
	edit: ClipEdit,
	id: string,
	pitchCents: number,
): ClipEdit {
	if (!Number.isFinite(pitchCents)) return edit;
	const segment = edit.segments.find((s) => s.id === id);
	if (!segment || segment.effects.pitchCents === pitchCents) return edit;
	return updateSegment(edit, id, {
		effects: { ...segment.effects, pitchCents },
	});
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

export const SNAP_EPSILON = 0.01;

/**
 * Applies an effects change to every selected segment and keeps the selected
 * group coherent: members that were snapped together move together, so a
 * contraction pulls its snapped selected follower left and an extension
 * carries it right, while unselected segments behave as normal obstacles -
 * pushed only when a growth actually reaches them, never pulled. Segments
 * that were not snapped keep their gaps.
 */
export function resizeSelectedSegments(
	edit: ClipEdit,
	ids: readonly string[],
	patch: (id: string, effects: SegmentEffects) => SegmentEffects,
): ClipEdit {
	const selected = ids
		.map((id) => edit.segments.find((s) => s.id === id))
		.filter((segment): segment is TimelineSegment => Boolean(segment));
	if (selected.length === 0) return edit;

	const oldEnds = new Map(
		selected.map((segment) => [segment.id, segmentEnd(segment)]),
	);
	const content = (segment: TimelineSegment) =>
		segment.sourceOut - segment.sourceIn;

	let next = edit;
	for (const segment of selected) {
		next = updateSegment(next, segment.id, {
			effects: patch(segment.id, segment.effects),
		});
	}

	// Snap the selected chain together per track, in timeline order. The
	// shift of one member is the end delta of the selected segment before
	// it; the snap decision itself always compares original positions, which
	// are not affected by the shifts.
	for (const track of new Set(selected.map((s) => s.track))) {
		const group = next.segments
			.filter((s) => ids.includes(s.id) && s.track === track)
			.sort((a, b) => a.timelineStart - b.timelineStart);
		if (group.length < 2) continue;
		let previousStart = group[0].timelineStart;
		for (let i = 1; i < group.length; i += 1) {
			const current = group[i];
			const previous = group[i - 1];
			const originalStart = current.timelineStart;
			const previousOldEnd = oldEnds.get(previous.id) ?? 0;
			const previousNewEnd =
				previousStart + content(previous) / effectiveRate(previous.effects);
			const start =
				Math.abs(originalStart - previousOldEnd) < SNAP_EPSILON
					? previousNewEnd
					: originalStart;
			if (start !== current.timelineStart) {
				next = updateSegment(next, current.id, { timelineStart: start });
			}
			previousStart = start;
		}
	}

	// Resolve the new layout per track: everything keeps its position unless
	// a growth reached it, in which case it is pushed to the new edge. A
	// shrinkage never pulls an unselected segment.
	for (const track of new Set(selected.map((s) => s.track))) {
		const row = next.segments
			.filter((s) => s.track === track)
			.sort((a, b) => a.timelineStart - b.timelineStart);
		let cursor = 0;
		for (const segment of row) {
			const start = Math.max(segment.timelineStart, cursor);
			if (start !== segment.timelineStart) {
				next = updateSegment(next, segment.id, { timelineStart: start });
			}
			cursor = Math.max(
				cursor,
				start + content(segment) / effectiveRate(segment.effects),
			);
		}
	}

	return next;
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
