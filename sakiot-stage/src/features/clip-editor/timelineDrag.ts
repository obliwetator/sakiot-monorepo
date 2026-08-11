import type { ClipEdit, TimelineSegment } from "./model";
import {
	leftEdgeFloor,
	MIN_SEGMENT_SECONDS,
	moveSegment,
	rightEdgeCeiling,
	segmentEnd,
	setSegmentRange,
	snapToNeighbors,
} from "./model";

const SNAP_SECONDS = 0.25;
const CLICK_THRESHOLD_PX = 4;

export type SegmentDragMode = "move" | "left" | "right";

export interface GroupedSegment {
	id: string;
	originStart: number;
	originTrack: number;
	/** Timeline duration of the segment, fixed during a move drag. */
	duration: number;
}

export interface SegmentDragState {
	mode: SegmentDragMode;
	segmentId: string;
	/** The dragged segment and every selected segment moved with it. */
	group: GroupedSegment[];
	originStart: number;
	originIn: number;
	originOut: number;
	originTrack: number;
	originRate: number;
	/** Fixed post-content duration processed through the effect chain. */
	originTail: number;
	/** The window plays backwards: edge trims eat the opposite source end. */
	reverse: boolean;
	maxSource: number;
	maxTrack: number;
	startX: number;
	startY: number;
	ghostStart: number;
	ghostIn: number;
	ghostOut: number;
	ghostTrack: number;
	/** Snapped starts of the group members besides the primary segment. */
	ghostStarts: Array<{ id: string; start: number }>;
	/** Ctrl/Cmd-click toggles the segment in the selection instead of
	 * replacing it; the commit must not collapse the already-toggled
	 * selection after a plain click. */
	modifierClick: boolean;
	/**
	 * The vertical move would push two members onto the same track with
	 * overlapping times: the whole gesture is blocked and warns.
	 */
	trackCollision: boolean;
	valid: boolean;
	/** An edge extension is being held back by a neighbouring clip. */
	clamped: boolean;
	pointerX: number;
	pointerY: number;
}

function snapTo(value: number, target: number): number {
	return Math.abs(value - target) < SNAP_SECONDS ? target : value;
}

/**
 * The group's shared timeline delta for a move drag: every member shifts by
 * the same amount so the selection stays rigid. The delta is the nearest
 * feasible one to `targetDelta` - feasible meaning no member drops below
 * zero and no member overlaps an unselected segment (touching a border is
 * allowed). The candidates are the target itself, the zero clamp, and every
 * boundary where a member would touch an obstacle.
 */
export function resolveGroupDelta(
	group: readonly GroupedSegment[],
	maxTrack: number,
	targetDelta: number,
	trackDelta: number,
	others: readonly TimelineSegment[],
): number {
	const minOriginStart = Math.min(...group.map((member) => member.originStart));
	const memberTrack = (member: GroupedSegment) =>
		Math.max(0, Math.min(maxTrack + 1, member.originTrack + trackDelta));
	const bounds: number[] = [];
	for (const member of group) {
		const track = memberTrack(member);
		for (const other of others) {
			if (other.track !== track) continue;
			// Touching the neighbour's start from the left.
			bounds.push(other.timelineStart - member.originStart - member.duration);
			// Touching the neighbour's end from the right.
			bounds.push(segmentEnd(other) - member.originStart);
		}
	}
	const feasible = (delta: number) =>
		group.every((member) => {
			if (member.originStart + delta < 0) return false;
			const track = memberTrack(member);
			const start = member.originStart + delta;
			const end = start + member.duration;
			return others.every(
				(other) =>
					other.track !== track ||
					end <= other.timelineStart ||
					start >= segmentEnd(other),
			);
		});
	// The zero clamp is the delta at which the leftmost member touches 0;
	// anything below it would push a member before the timeline start.
	const candidates = [targetDelta, 0 - minOriginStart, ...bounds].sort(
		(a, b) => Math.abs(a - targetDelta) - Math.abs(b - targetDelta),
	);
	return candidates.find(feasible) ?? targetDelta;
}

/**
 * Clamps a viewport coordinate pair into a rectangle (e.g. the tracks
 * container) so a marquee started inside it never renders outside its bounds
 * when the pointer leaves the container.
 */
export function clampPointToRect(
	pointX: number,
	pointY: number,
	rect: { left: number; right: number; top: number; bottom: number },
): { x: number; y: number } {
	return {
		x: Math.min(Math.max(pointX, rect.left), rect.right),
		y: Math.min(Math.max(pointY, rect.top), rect.bottom),
	};
}

/**
 * The marquee overlay's CSS offset inside the tracks container. The overlay
 * is an absolutely positioned child of the scroll container, so its offsets
 * are relative to the scrolled content: the visible-viewport coordinate must
 * be shifted back by the scroll position, or the box drifts away from the
 * pointer whenever the container is scrolled.
 */
export function marqueeOverlayOffset(
	startX: number,
	startY: number,
	currentX: number,
	currentY: number,
	rect: { left: number; top: number },
	scrollLeft: number,
	scrollTop: number,
): { left: number; top: number } {
	return {
		left: Math.min(startX, currentX) - rect.left + scrollLeft,
		top: Math.min(startY, currentY) - rect.top + scrollTop,
	};
}

/**
 * Whether a marquee rectangle (viewport bounds) touches a segment's row box.
 * The row's vertical padding mirrors the segment boxes drawn inside the row.
 */
export function marqueeIntersectsSegment(
	startFraction: number,
	endFraction: number,
	rowRect: {
		left: number;
		right: number;
		top: number;
		bottom: number;
		width: number;
	},
	bounds: { left: number; right: number; top: number; bottom: number },
): boolean {
	if (rowRect.width <= 0) return false;
	const segmentLeft = rowRect.left + (startFraction / 100) * rowRect.width;
	const segmentRight = rowRect.left + (endFraction / 100) * rowRect.width;
	const segmentTop = rowRect.top + 8;
	const segmentBottom = rowRect.bottom - 8;
	return (
		bounds.left < segmentRight &&
		bounds.right > segmentLeft &&
		bounds.top < segmentBottom &&
		bounds.bottom > segmentTop
	);
}

/**
 * Whether the vertical move would push two members onto the same track with
 * overlapping timeline ranges. Members move by one shared delta, so their
 * relative times never change: the check compares the origin ranges. Moving
 * up can merge tracks (the top member clamps at track 0), which squishes
 * tracks that were separate; moving down only ever lands on tracks above
 * every origin, so it can never merge two members.
 */
export function groupTrackCollision(
	group: readonly GroupedSegment[],
	trackDelta: number,
): boolean {
	// Only the lower clamp matters: the commit moves members to
	// max(0, originTrack + delta), growing the track count as needed, so
	// there is no upper bound to collide against.
	const targets = group.map((member) => ({
		member,
		track: Math.max(0, member.originTrack + trackDelta),
	}));
	for (let i = 0; i < targets.length; i += 1) {
		for (let j = i + 1; j < targets.length; j += 1) {
			if (targets[i].track !== targets[j].track) continue;
			const a = targets[i].member;
			const b = targets[j].member;
			if (
				a.originStart < b.originStart + b.duration &&
				b.originStart < a.originStart + a.duration
			) {
				return true;
			}
		}
	}
	return false;
}

function computeGhostState(
	drag: SegmentDragState,
	pointer: TimelinePointerInput,
	pxPerSec: number,
	positionSec: number,
	segments: readonly TimelineSegment[],
	track: number,
): SegmentDragState {
	const dt = (pointer.clientX - drag.startX) / Math.max(0.0001, pxPerSec);
	// A press without real travel is a click: return the untouched state so
	// the commit can tell clicks from drags (e.g. collapsing a selection).
	if (
		Math.abs(pointer.clientX - drag.startX) < CLICK_THRESHOLD_PX &&
		Math.abs(pointer.clientY - drag.startY) < CLICK_THRESHOLD_PX
	) {
		return drag;
	}
	if (drag.mode === "move") {
		const rect = pointer.containerRect;
		const valid = rect
			? pointer.clientY >= rect.top && pointer.clientY <= rect.bottom
			: true;
		const rawStart = Math.max(0, drag.originStart + dt);
		// The target track is the row under the pointer (extrapolated past
		// the last row), so scrolling the container mid-drag never desyncs
		// the ghost from the pointer. No upper clamp: one long drag can
		// create as many new tracks as it reaches.
		const ghostTrack = Math.max(0, track);
		const trackDelta = ghostTrack - drag.originTrack;
		const groupIds = new Set(drag.group.map((member) => member.id));
		const primary = drag.group.find((member) => member.id === drag.segmentId);
		// The primary drives the target position (playhead snap included);
		// unselected segments are the only obstacles, and the whole group
		// shifts by one shared delta so the selection stays rigid.
		const others = segments.filter((segment) => !groupIds.has(segment.id));
		// A vertical move that would squash two tracks together cannot be
		// done: the gesture is rejected and the ghosts stay put.
		const trackCollision = groupTrackCollision(drag.group, trackDelta);
		if (trackCollision) {
			return {
				...drag,
				ghostStart: drag.originStart,
				ghostStarts: drag.group.map((member) => ({
					id: member.id,
					start: member.originStart,
				})),
				ghostTrack,
				valid,
				clamped: false,
				trackCollision,
				pointerX: pointer.clientX,
				pointerY: pointer.clientY,
			};
		}
		const targetStart = snapToNeighbors(
			snapTo(rawStart, positionSec),
			primary?.duration ??
				(drag.originOut - drag.originIn) / drag.originRate + drag.originTail,
			others,
			"",
			ghostTrack,
			rawStart,
		);
		const delta = resolveGroupDelta(
			drag.group,
			drag.maxTrack,
			targetStart - drag.originStart,
			trackDelta,
			others,
		);
		const ghostStarts = drag.group.map((member) => ({
			id: member.id,
			start: member.originStart + delta,
		}));
		const primaryStart =
			ghostStarts.find((member) => member.id === drag.segmentId)?.start ??
			targetStart;
		return {
			...drag,
			ghostStart: primaryStart,
			ghostStarts,
			ghostTrack,
			valid,
			clamped: false,
			trackCollision: false,
			pointerX: pointer.clientX,
			pointerY: pointer.clientY,
		};
	}
	if (drag.mode === "left") {
		if (drag.reverse) {
			// The left box edge plays the source-window end, so trimming it
			// eats content nearest sourceOut and extending it grows toward
			// the end of the clip (maxSource) while sourceIn stays put.
			const rawGhostOut = Math.max(
				drag.originIn + MIN_SEGMENT_SECONDS,
				Math.min(drag.maxSource, drag.originOut - dt * drag.originRate),
			);
			const snappedStart = snapTo(
				drag.originStart + (drag.originOut - rawGhostOut) / drag.originRate,
				positionSec,
			);
			const ghostOut = Math.max(
				drag.originIn + MIN_SEGMENT_SECONDS,
				Math.min(
					drag.maxSource,
					drag.originOut - (snappedStart - drag.originStart) * drag.originRate,
				),
			);
			const ghostStart =
				drag.originStart + (drag.originOut - ghostOut) / drag.originRate;
			const originEnd =
				drag.originStart +
				(drag.originOut - drag.originIn) / drag.originRate +
				drag.originTail;
			const clampedStart = Math.max(
				ghostStart,
				leftEdgeFloor(segments, drag.originTrack, drag.segmentId, originEnd),
			);
			return {
				...drag,
				ghostOut:
					clampedStart === ghostStart
						? ghostOut
						: drag.originOut -
							(clampedStart - drag.originStart) * drag.originRate,
				ghostStart: clampedStart,
				clamped: clampedStart !== ghostStart,
				pointerX: pointer.clientX,
				pointerY: pointer.clientY,
			};
		}
		const rawGhostIn = Math.max(
			0,
			Math.min(
				drag.originOut - MIN_SEGMENT_SECONDS,
				drag.originIn + dt * drag.originRate,
			),
		);
		const snappedStart = snapTo(
			drag.originStart + (rawGhostIn - drag.originIn) / drag.originRate,
			positionSec,
		);
		const ghostIn = Math.max(
			0,
			Math.min(
				drag.originOut - MIN_SEGMENT_SECONDS,
				drag.originIn + (snappedStart - drag.originStart) * drag.originRate,
			),
		);
		const ghostStart =
			drag.originStart + (ghostIn - drag.originIn) / drag.originRate;
		// The box end stays fixed during a left-edge drag, so extending the
		// start left is bounded by the nearest neighbour's end.
		const originEnd =
			drag.originStart +
			(drag.originOut - drag.originIn) / drag.originRate +
			drag.originTail;
		const clampedStart = Math.max(
			ghostStart,
			leftEdgeFloor(segments, drag.originTrack, drag.segmentId, originEnd),
		);
		return {
			...drag,
			ghostIn:
				clampedStart === ghostStart
					? ghostIn
					: drag.originIn + (clampedStart - drag.originStart) * drag.originRate,
			ghostStart: clampedStart,
			clamped: clampedStart !== ghostStart,
			pointerX: pointer.clientX,
			pointerY: pointer.clientY,
		};
	}
	if (drag.mode === "right" && drag.reverse) {
		// The right box edge plays the source-window start, so trimming
		// it eats content nearest sourceIn; extending it reaches toward
		// the start of the clip (sourceIn >= 0) while sourceOut stays.
		const rawGhostIn = Math.max(
			0,
			Math.min(
				drag.originOut - MIN_SEGMENT_SECONDS,
				drag.originIn - dt * drag.originRate,
			),
		);
		const endSec =
			drag.originStart +
			(drag.originOut - drag.originIn) / drag.originRate +
			drag.originTail;
		const snappedEnd = snapTo(
			endSec - (rawGhostIn - drag.originIn) / drag.originRate,
			positionSec,
		);
		const ghostIn = Math.max(
			0,
			Math.min(
				drag.originOut - MIN_SEGMENT_SECONDS,
				drag.originIn - (snappedEnd - endSec) * drag.originRate,
			),
		);
		// The box start stays fixed during a right-edge drag, so
		// extending the end right is bounded by the neighbour's start.
		const ceiling = rightEdgeCeiling(
			segments,
			drag.originTrack,
			drag.segmentId,
			drag.originStart,
		);
		const minGhostIn =
			ceiling === null
				? 0
				: drag.originOut -
					Math.max(0, ceiling - drag.originStart - drag.originTail) *
						drag.originRate;
		const clampedGhostIn = Math.max(ghostIn, minGhostIn);
		return {
			...drag,
			ghostIn: clampedGhostIn,
			clamped: clampedGhostIn > ghostIn,
			pointerX: pointer.clientX,
			pointerY: pointer.clientY,
		};
	}
	const rawGhostOut = Math.max(
		drag.originIn + MIN_SEGMENT_SECONDS,
		Math.min(drag.maxSource, drag.originOut + dt * drag.originRate),
	);
	const endSec =
		drag.originStart +
		(drag.originOut - drag.originIn) / drag.originRate +
		drag.originTail;
	const snappedEnd = snapTo(
		endSec + (rawGhostOut - drag.originOut) / drag.originRate,
		positionSec,
	);
	const ghostOut = Math.min(
		drag.maxSource,
		drag.originOut + (snappedEnd - endSec) * drag.originRate,
	);
	// The box start stays fixed during a right-edge drag, so extending the
	// end right is bounded by the nearest neighbour's start.
	const ceiling = rightEdgeCeiling(
		segments,
		drag.originTrack,
		drag.segmentId,
		drag.originStart,
	);
	const maxGhostOut =
		ceiling === null
			? drag.maxSource
			: drag.originIn +
				Math.max(0, ceiling - drag.originStart - drag.originTail) *
					drag.originRate;
	const clampedGhostOut = Math.min(ghostOut, maxGhostOut);
	return {
		...drag,
		ghostOut: clampedGhostOut,
		clamped: clampedGhostOut < ghostOut,
		pointerX: pointer.clientX,
		pointerY: pointer.clientY,
	};
}

export interface TimelinePointerInput {
	clientX: number;
	clientY: number;
	containerRect: { top: number; bottom: number; height: number } | null;
}

export interface TimelineDragTransition {
	state: SegmentDragState;
	scrollDeltaY: number;
}

export function timelineScrollRequest(pointer: TimelinePointerInput): number {
	const rect = pointer.containerRect;
	if (!rect || pointer.clientY < rect.top || pointer.clientY > rect.bottom)
		return 0;
	const edge = Math.max(48, rect.height * 0.2);
	if (pointer.clientY > rect.bottom - edge) {
		return Math.max(8, (pointer.clientY - (rect.bottom - edge)) * 2);
	}
	if (pointer.clientY < rect.top + edge) {
		return -Math.max(8, (rect.top + edge - pointer.clientY) * 2);
	}
	return 0;
}

export function transitionTimelineDrag(
	drag: SegmentDragState,
	pointer: TimelinePointerInput,
	pxPerSec: number,
	positionSec: number,
	segments: readonly TimelineSegment[],
	track: number,
): TimelineDragTransition {
	return {
		state: computeGhostState(
			drag,
			pointer,
			pxPerSec,
			positionSec,
			segments,
			track,
		),
		scrollDeltaY: drag.mode === "move" ? timelineScrollRequest(pointer) : 0,
	};
}

export function dragGhostGeometry(
	drag: SegmentDragState,
	member: GroupedSegment,
): { startSec: number; endSec: number } {
	if (drag.mode === "move") {
		const ghost = drag.ghostStarts.find((g) => g.id === member.id);
		const startSec = ghost?.start ?? member.originStart;
		return { startSec, endSec: startSec + member.duration };
	}
	if (drag.mode === "left") {
		// The box end stays fixed while the start edge moves.
		return {
			startSec: drag.ghostStart,
			endSec: drag.originStart + member.duration,
		};
	}
	// Right-edge drag: the box start stays fixed while the end edge moves.
	return {
		startSec: drag.originStart,
		endSec:
			drag.originStart +
			(drag.ghostOut - drag.ghostIn) / drag.originRate +
			drag.originTail,
	};
}

/** Ghost geometries of every segment dragged as a group. */
export function dragGhostGeometries(drag: SegmentDragState): Array<{
	segmentId: string;
	startSec: number;
	endSec: number;
	track: number;
}> {
	if (drag.mode !== "move") {
		const primary = drag.group.find((member) => member.id === drag.segmentId);
		if (!primary) return [];
		const geometry = dragGhostGeometry(drag, primary);
		return [{ segmentId: drag.segmentId, ...geometry, track: drag.ghostTrack }];
	}
	const trackDelta = drag.ghostTrack - drag.originTrack;
	return drag.group.map((member) => {
		const geometry = dragGhostGeometry(drag, member);
		return {
			segmentId: member.id,
			...geometry,
			track: Math.max(0, member.originTrack + trackDelta),
		};
	});
}

export function applySegmentDrag(
	edit: ClipEdit,
	drag: SegmentDragState,
): ClipEdit {
	if (drag.mode === "move") {
		if (!edit.segments.some((segment) => segment.id === drag.segmentId))
			return edit;
		const trackDelta = drag.ghostTrack - drag.originTrack;
		let next = edit;
		let maxTrack = edit.tracks - 1;
		for (const member of drag.group) {
			const ghost = drag.ghostStarts.find(
				(candidate) => candidate.id === member.id,
			);
			const start = ghost?.start ?? member.originStart;
			const track = Math.max(0, member.originTrack + trackDelta);
			next = moveSegment(next, member.id, start, track);
			maxTrack = Math.max(maxTrack, track);
		}
		return maxTrack >= edit.tracks ? { ...next, tracks: maxTrack + 1 } : next;
	}
	if (drag.mode === "left") {
		if (!edit.segments.some((segment) => segment.id === drag.segmentId))
			return edit;
		const ranged = setSegmentRange(
			edit,
			drag.segmentId,
			drag.ghostIn,
			drag.ghostOut,
		);
		return moveSegment(
			ranged,
			drag.segmentId,
			drag.ghostStart,
			drag.ghostTrack,
		);
	}
	if (!edit.segments.some((segment) => segment.id === drag.segmentId))
		return edit;
	return setSegmentRange(edit, drag.segmentId, drag.ghostIn, drag.ghostOut);
}
