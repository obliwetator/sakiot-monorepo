import ZoomInIcon from "@mui/icons-material/ZoomIn";
import ZoomOutIcon from "@mui/icons-material/ZoomOut";
import Box from "@mui/material/Box";
import IconButton from "@mui/material/IconButton";
import { keyframes } from "@mui/material/styles";
import Tooltip from "@mui/material/Tooltip";
import Typography from "@mui/material/Typography";
import type {
	DragEvent as ReactDragEvent,
	PointerEvent as ReactPointerEvent,
} from "react";
import { useEffect, useRef, useState } from "react";
import { usePointerDrag } from "../../shared/pointerDrag";
import { formatDuration } from "../../utils/formatTime";
import {
	axisLabelTransform,
	gridLineOffset,
	TIMELINE_AXIS_FRACTIONS,
	TimelinePlayhead,
	TimelineRow,
} from "../audio-dashboard/timelineLayout";
import { pendingBinDrag } from "./ClipBin";
import type { TimelineSegment } from "./model";
import {
	effectiveRate,
	leftEdgeFloor,
	MIN_SEGMENT_SECONDS,
	moveSegment,
	rightEdgeCeiling,
	segmentDuration,
	segmentEnd,
	setSegmentRange,
	snapToNeighbors,
} from "./model";
import { SegmentWaveform } from "./SegmentWaveform";
import type { UseClipEditorReturn } from "./useClipEditor";
import { useClipWaveform } from "./useClipWaveform";
import { useProcessedSegmentWaveform } from "./useProcessedSegmentWaveform";

const TRACK_HEIGHT_PX = 72;
const HANDLE_WIDTH_PX = 7;
const SNAP_SECONDS = 0.25;
/** Pointer travel below this counts as a click, not a drag. */
const CLICK_THRESHOLD_PX = 4;
/** Dashes marching clockwise around the ring of the segment just copied. */
const copiedDashes = keyframes`
	from { stroke-dashoffset: 0; }
	to { stroke-dashoffset: -18; }
`;

interface DraggedClipPayload {
	clipId: string;
	lengthSec: number;
}

interface DragPreviewState {
	clipId: string;
	lengthSec: number;
	/** Existing row index, or `edit.tracks` when the drop would add a track. */
	track: number;
	startSec: number;
}

type SegmentDragMode = "move" | "left" | "right";

/** A live marquee; the rectangle spans the tracks the drag crosses. */
interface MarqueeState {
	startX: number;
	startY: number;
	currentX: number;
	currentY: number;
	track: number;
}

interface GroupedSegment {
	id: string;
	originStart: number;
	originTrack: number;
	/** Timeline duration of the segment, fixed during a move drag. */
	duration: number;
}

interface SegmentDragState {
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

function parseDraggedClip(
	dataTransfer: DataTransfer,
): DraggedClipPayload | null {
	const pending = pendingBinDrag.payload;
	if (pending) return pending;
	const raw = dataTransfer.getData("application/json");
	if (!raw) return null;
	try {
		const parsed = JSON.parse(raw) as { clip_id?: unknown; length?: unknown };
		if (typeof parsed.clip_id !== "string") return null;
		const lengthSec = typeof parsed.length === "number" ? parsed.length : 0;
		return { clipId: parsed.clip_id, lengthSec };
	} catch {
		return null;
	}
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

function computeGhost(
	drag: SegmentDragState,
	event: PointerEvent,
	pxPerSec: number,
	positionSec: number,
	container: HTMLElement | null,
	segments: readonly TimelineSegment[],
	trackAtClientY: (clientY: number) => number,
): SegmentDragState {
	const dt = (event.clientX - drag.startX) / Math.max(0.0001, pxPerSec);
	// A press without real travel is a click: return the untouched state so
	// the commit can tell clicks from drags (e.g. collapsing a selection).
	if (
		Math.abs(event.clientX - drag.startX) < CLICK_THRESHOLD_PX &&
		Math.abs(event.clientY - drag.startY) < CLICK_THRESHOLD_PX
	) {
		return drag;
	}
	if (drag.mode === "move") {
		const rect = container?.getBoundingClientRect();
		const valid = rect
			? event.clientY >= rect.top && event.clientY <= rect.bottom
			: true;
		// Edge auto-scroll: dragging near the top or bottom edge of the track
		// area scrolls it, so long vertical moves reach tracks below the fold
		// without the pointer leaving the drag. The zone covers 20% of the
		// visible height (never less than 48px).
		if (container && rect) {
			const edge = Math.max(48, rect.height * 0.2);
			if (event.clientY >= rect.top && event.clientY <= rect.bottom) {
				if (event.clientY > rect.bottom - edge) {
					container.scrollTop += Math.max(
						8,
						(event.clientY - (rect.bottom - edge)) * 2,
					);
				} else if (event.clientY < rect.top + edge) {
					container.scrollTop -= Math.max(
						8,
						(rect.top + edge - event.clientY) * 2,
					);
				}
			}
		}
		const rawStart = Math.max(0, drag.originStart + dt);
		// The target track is the row under the pointer (extrapolated past
		// the last row), so scrolling the container mid-drag never desyncs
		// the ghost from the pointer. No upper clamp: one long drag can
		// create as many new tracks as it reaches.
		const ghostTrack = Math.max(0, trackAtClientY(event.clientY));
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
				pointerX: event.clientX,
				pointerY: event.clientY,
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
			pointerX: event.clientX,
			pointerY: event.clientY,
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
				pointerX: event.clientX,
				pointerY: event.clientY,
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
			pointerX: event.clientX,
			pointerY: event.clientY,
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
			pointerX: event.clientX,
			pointerY: event.clientY,
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
		pointerX: event.clientX,
		pointerY: event.clientY,
	};
}

function dragGhostGeometry(
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
function dragGhostGeometries(drag: SegmentDragState): Array<{
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

export function Timeline(props: {
	guildId: string;
	editor: UseClipEditorReturn;
	clipName: (segment: TimelineSegment) => string;
	onDropClip: (
		clipId: string,
		lengthSec: number,
		track: number,
		startSec: number,
	) => void;
	/** Marquee selection covers every track the rectangle spans. */
	multiTrackMarquee?: boolean;
}) {
	const { editor } = props;
	const plotRef = useRef<HTMLDivElement | null>(null);
	const tracksRef = useRef<HTMLDivElement | null>(null);
	const rowElementsRef = useRef(new Map<number, HTMLElement>());
	const [plotWidth, setPlotWidth] = useState(1);
	const [preview, setPreview] = useState<DragPreviewState | null>(null);
	const segmentDrag = usePointerDrag<SegmentDragState>({
		compute: (ghost, event) =>
			computeGhost(
				ghost,
				event,
				pxPerSec,
				editor.positionSec,
				tracksRef.current,
				editor.edit.segments,
				trackAtClientY,
			),
		onCommit: ({ ghost, origin }) => {
			// A rejected vertical move must not commit or touch the selection.
			if (ghost.trackCollision) return;
			if (ghost !== origin && ghost.valid) {
				commitSegmentDrag(ghost, editor.apply);
				return;
			}
			// A click on a segment inside a multi-selection collapses the
			// selection to that segment (pointerdown kept the group so a
			// drag can move it together). Ctrl/Cmd clicks toggled the segment
			// already, so they must not collapse.
			if (!ghost.modifierClick) {
				editor.select(ghost.segmentId);
			}
		},
	});
	const segmentDragGhost = segmentDrag.snapshot?.ghost ?? null;

	const lastMarqueeSelectionRef = useRef<string[]>([]);

	/** Segments the marquee rectangle touches, optionally across all tracks. */
	const marqueeOverlapIds = (state: MarqueeState) => {
		const left = Math.min(state.startX, state.currentX);
		const right = Math.max(state.startX, state.currentX);
		const top = Math.min(state.startY, state.currentY);
		const bottom = Math.max(state.startY, state.currentY);
		const bounds = { left, right, top, bottom };
		return editor.edit.segments
			.filter(
				(segment) => props.multiTrackMarquee || segment.track === state.track,
			)
			.filter((segment) => {
				const rowRect = rowElementsRef.current
					.get(segment.track)
					?.getBoundingClientRect();
				if (!rowRect) return false;
				return marqueeIntersectsSegment(
					fraction(segment.timelineStart),
					fraction(segmentEnd(segment)),
					rowRect,
					bounds,
				);
			})
			.map((segment) => segment.id);
	};

	const marquee = usePointerDrag<MarqueeState>({
		compute: (ghost, event) => {
			// Keep the rectangle inside the tracks container: dragging toward
			// the screen corner would otherwise push it off its top-left edge
			// (negative offsets), rendering the box out of bounds.
			const container = tracksRef.current;
			const rect = container?.getBoundingClientRect();
			const point = rect
				? clampPointToRect(event.clientX, event.clientY, rect)
				: { x: event.clientX, y: event.clientY };
			const next = {
				...ghost,
				currentX: point.x,
				currentY: point.y,
			};
			const ids = marqueeOverlapIds(next);
			if (ids.length !== lastMarqueeSelectionRef.current.length) {
				lastMarqueeSelectionRef.current = ids;
				editor.selectMany(ids);
			}
			return next;
		},
		onCommit: () => {},
	});

	// Escape aborts a live marquee so a stuck or unwanted box never lingers.
	const marqueeRef = useRef(marquee);
	useEffect(() => {
		marqueeRef.current = marquee;
	}, [marquee]);
	useEffect(() => {
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.key === "Escape" && marqueeRef.current.isDragging) {
				marqueeRef.current.cancel();
			}
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, []);

	// A click-drag on a track's empty space starts a live marquee: the track
	// becomes active and the previous selection is replaced as the rectangle
	// crosses segment borders.
	const beginMarquee = (
		event: ReactPointerEvent<HTMLElement>,
		track: number,
	) => {
		if (event.button !== 0) return;
		event.preventDefault();
		// Capture the pointer so the release is always delivered - without it,
		// letting go outside the window misses the pointerup and leaves the
		// marquee stuck following the mouse. Captured events bubble to the
		// window listeners the drag hook relies on.
		try {
			event.currentTarget.setPointerCapture(event.pointerId);
		} catch {
			// Best-effort: the window listeners still cover the usual drag.
		}
		editor.selectTrack(track);
		lastMarqueeSelectionRef.current = [];
		editor.selectMany([]);
		marquee.begin(
			{
				startX: event.clientX,
				startY: event.clientY,
				currentX: event.clientX,
				currentY: event.clientY,
				track,
			},
			event.clientX,
			event.clientY,
		);
	};

	useEffect(() => {
		const plot = plotRef.current;
		if (!plot) return;
		const measure = () => {
			for (const element of rowElementsRef.current.values()) {
				const rect = element.getBoundingClientRect();
				if (rect.width > 0) {
					setPlotWidth(rect.width);
					return;
				}
			}
			setPlotWidth(plot.clientWidth);
		};
		measure();
		const observer = new ResizeObserver(measure);
		observer.observe(plot);
		return () => observer.disconnect();
	}, []);

	const fraction = (sec: number) =>
		((sec - editor.viewStartSec) / Math.max(1, editor.viewWidthSec)) * 100;

	const pxPerSec = plotWidth / Math.max(1, editor.viewWidthSec);
	const rows = Array.from({ length: editor.edit.tracks }, (_, track) => track);

	const findTrackAtY = (clientY: number): number | null => {
		let best: { track: number; dist: number } | null = null;
		for (const [track, element] of rowElementsRef.current) {
			const rect = element.getBoundingClientRect();
			const mid = (rect.top + rect.bottom) / 2;
			const dist = Math.abs(clientY - mid);
			if (!best || dist < best.dist) best = { track, dist };
		}
		if (!best || best.dist > TRACK_HEIGHT_PX) return null;
		return best.track;
	};

	/**
	 * The row under the pointer, extrapolated past the last rendered row with
	 * the real row pitch. Rects are viewport-relative, so this stays correct
	 * no matter how much the track area is scrolled mid-drag.
	 */
	const trackAtClientY = (clientY: number): number => {
		let nearest: { track: number; dist: number } | null = null;
		let lastTop = 0;
		let lastBottom = 0;
		let lastTrack = 0;
		for (const [track, element] of rowElementsRef.current) {
			const rect = element.getBoundingClientRect();
			lastTop = rect.top;
			lastBottom = rect.bottom;
			lastTrack = track;
			const dist = Math.abs(clientY - (rect.top + rect.bottom) / 2);
			if (!nearest || dist < nearest.dist) nearest = { track, dist };
		}
		if (!nearest) return 0;
		if (nearest.dist <= TRACK_HEIGHT_PX / 2) return nearest.track;
		// Beyond the last rendered row (e.g. the new-track area): extrapolate
		// from the last row's pitch (row height plus its margin).
		const pitch = lastBottom - lastTop + 4;
		const beyond = Math.round((clientY - (lastTop + lastBottom) / 2) / pitch);
		return Math.max(0, lastTrack + beyond);
	};

	// Ghosts and segments render inside the plot area (after the label gutter),
	// so cursor-to-time mapping must use a track row's rect, not the container.
	const currentPlotBounds = () => {
		for (const element of rowElementsRef.current.values()) {
			const rect = element.getBoundingClientRect();
			if (rect.width > 0) return rect;
		}
		return tracksRef.current?.getBoundingClientRect() ?? { left: 0, width: 1 };
	};

	const computeDrop = (clientX: number, clientY: number, lengthSec: number) => {
		const plotBounds = currentPlotBounds();
		const found = findTrackAtY(clientY);
		const track =
			found === null ? editor.edit.tracks : Math.min(found, editor.edit.tracks);
		const fractionOfWidth = Math.min(
			1,
			Math.max(0, (clientX - plotBounds.left) / Math.max(1, plotBounds.width)),
		);
		const rawStart =
			editor.viewStartSec + fractionOfWidth * editor.viewWidthSec;
		let startSec = rawStart;
		if (Math.abs(startSec - editor.positionSec) < SNAP_SECONDS) {
			startSec = editor.positionSec;
		}
		startSec = snapToNeighbors(
			startSec,
			lengthSec,
			editor.edit.segments,
			"",
			track,
			rawStart,
		);
		return { track, startSec: Math.max(0, startSec) };
	};

	const handleDragOver = (event: ReactDragEvent<HTMLElement>) => {
		event.preventDefault();
		event.dataTransfer.dropEffect = "copy";
		const payload = parseDraggedClip(event.dataTransfer);
		if (!payload) return;
		const { track, startSec } = computeDrop(
			event.clientX,
			event.clientY,
			payload.lengthSec,
		);
		setPreview((previous) =>
			previous &&
			previous.clipId === payload.clipId &&
			previous.track === track &&
			previous.startSec === startSec
				? previous
				: {
						clipId: payload.clipId,
						lengthSec: payload.lengthSec,
						track,
						startSec,
					},
		);
	};

	const handleDrop = (event: ReactDragEvent<HTMLElement>) => {
		event.preventDefault();
		const payload = parseDraggedClip(event.dataTransfer);
		setPreview(null);
		if (!payload) return;
		const { track, startSec } = computeDrop(
			event.clientX,
			event.clientY,
			payload.lengthSec,
		);
		props.onDropClip(payload.clipId, payload.lengthSec, track, startSec);
	};

	const handleDragLeave = (event: ReactDragEvent<HTMLElement>) => {
		if (!event.currentTarget.contains(event.relatedTarget as Node)) {
			setPreview(null);
		}
	};

	/** Ghost previews of the dragged group landing on a given track. */
	const ghostGeometriesForTrack = (track: number) =>
		segmentDragGhost
			? dragGhostGeometries(segmentDragGhost)
					.filter((geometry) => geometry.track === track)
					.map((geometry) => {
						const dragged = editor.edit.segments.find(
							(s) => s.id === geometry.segmentId,
						);
						return {
							segmentId: geometry.segmentId,
							leftFraction: fraction(geometry.startSec),
							widthFraction: Math.max(
								0,
								fraction(geometry.endSec) - fraction(geometry.startSec),
							),
							name: dragged ? props.clipName(dragged) : "",
							invalid: false,
						};
					})
			: [];

	return (
		<Box
			component="section"
			aria-label="Clip editor timeline"
			sx={{
				display: "flex",
				flexDirection: "column",
				minHeight: 0,
				minWidth: 0,
				height: "100%",
				borderTop: 1,
				borderColor: "divider",
				p: 1,
				overflow: "hidden",
			}}
		>
			<Box
				ref={plotRef}
				sx={{
					display: "flex",
					flexDirection: "column",
					flex: 1,
					minHeight: 0,
					minWidth: 0,
				}}
			>
				<TimelineRuler
					fraction={fraction}
					positionSec={editor.positionSec}
					onScrub={editor.setPosition}
					viewStartSec={editor.viewStartSec}
					viewWidthSec={editor.viewWidthSec}
				/>
				<Box
					ref={tracksRef}
					onDragOver={handleDragOver}
					onDrop={handleDrop}
					onDragLeave={handleDragLeave}
					sx={{
						flex: 1,
						minHeight: 0,
						overflowY: "auto",
						display: "flex",
						flexDirection: "column",
						position: "relative",
					}}
				>
					{(() => {
						const marqueeState = marquee.snapshot?.ghost;
						const container = tracksRef.current;
						if (!marqueeState || !container) return null;
						const rect = container.getBoundingClientRect();
						const offset = marqueeOverlayOffset(
							marqueeState.startX,
							marqueeState.startY,
							marqueeState.currentX,
							marqueeState.currentY,
							rect,
							container.scrollLeft,
							container.scrollTop,
						);
						return (
							<Box
								aria-hidden="true"
								// Per-frame geometry goes inline: emotion would
								// otherwise inject a new <style> tag on every
								// pointermove and never remove the old ones.
								style={{
									position: "absolute",
									left: offset.left,
									top: offset.top,
									width: Math.abs(marqueeState.currentX - marqueeState.startX),
									height: Math.abs(marqueeState.currentY - marqueeState.startY),
								}}
								sx={{
									border: "1px dashed",
									borderColor: "primary.light",
									bgcolor: "rgba(56, 189, 248, 0.08)",
									pointerEvents: "none",
									zIndex: 20,
								}}
							/>
						);
					})()}
					{rows.map((track) => {
						return (
							<TrackRow
								key={track}
								track={track}
								guildId={props.guildId}
								editor={editor}
								clipName={props.clipName}
								fraction={fraction}
								pxPerSec={pxPerSec}
								preview={preview}
								active={editor.activeTrack === track}
								onActivate={() => editor.selectTrack(track)}
								dragGhosts={ghostGeometriesForTrack(track)}
								draggingSegmentIds={
									segmentDragGhost
										? dragGhostGeometries(segmentDragGhost).map(
												(geometry) => geometry.segmentId,
											)
										: []
								}
								onRowRef={(element) => {
									if (element) rowElementsRef.current.set(track, element);
									else rowElementsRef.current.delete(track);
								}}
								onBeginSegmentDrag={(drag) =>
									segmentDrag.begin(drag, drag.startX, drag.startY)
								}
								onBeginMarquee={beginMarquee}
							/>
						);
					})}
					{segmentDragGhost?.valid &&
						(() => {
							// One phantom row per new track the group lands on,
							// so a wide vertical move previews every track it
							// would create.
							const tracks = [
								...new Set(
									dragGhostGeometries(segmentDragGhost)
										.map((geometry) => geometry.track)
										.filter((track) => track >= editor.edit.tracks),
								),
							].sort((a, b) => a - b);
							if (tracks.length === 0) return null;
							return tracks.map((track) => (
								<PhantomTrackRow
									key={track}
									label={`Track ${track + 1}`}
									ghosts={ghostGeometriesForTrack(track).map((geometry) => ({
										key: geometry.segmentId,
										leftFraction: geometry.leftFraction,
										widthFraction: geometry.widthFraction,
										name: geometry.name,
										invalid: geometry.invalid,
									}))}
								/>
							));
						})()}
					{preview && preview.track === editor.edit.tracks && (
						<PhantomTrackRow
							label={`Track ${editor.edit.tracks + 1}`}
							ghosts={[
								{
									key: "bin-drop",
									leftFraction: fraction(preview.startSec),
									widthFraction: Math.max(
										0,
										fraction(preview.startSec + preview.lengthSec) -
											fraction(preview.startSec),
									),
									name: "",
									invalid: false,
								},
							]}
						/>
					)}
				</Box>
			</Box>
			{segmentDragGhost && !segmentDragGhost.valid && (
				<FloatingDragChip
					name={clipNameOfDragged(segmentDragGhost, editor, props.clipName)}
					x={segmentDragGhost.pointerX}
					y={segmentDragGhost.pointerY}
				/>
			)}
			{segmentDragGhost?.clamped && (
				<ClampedEdgeWarning
					x={segmentDragGhost.pointerX}
					y={segmentDragGhost.pointerY}
				/>
			)}
			{segmentDragGhost?.trackCollision && (
				<TrackCollisionWarning
					x={segmentDragGhost.pointerX}
					y={segmentDragGhost.pointerY}
				/>
			)}
			<Box sx={{ display: "flex", alignItems: "center", gap: 1, mt: 1 }}>
				<Tooltip title="Zoom out">
					<IconButton size="small" onClick={() => editor.zoom(2)}>
						<ZoomOutIcon fontSize="small" />
					</IconButton>
				</Tooltip>
				<Tooltip title="Fit edit in view">
					<IconButton size="small" onClick={editor.fitView}>
						<Typography variant="caption" sx={{ px: 0.5 }}>
							Fit
						</Typography>
					</IconButton>
				</Tooltip>
				<Tooltip title="Zoom in">
					<IconButton size="small" onClick={() => editor.zoom(0.5)}>
						<ZoomInIcon fontSize="small" />
					</IconButton>
				</Tooltip>
				<Typography
					variant="caption"
					color="text.secondary"
					sx={{ fontVariantNumeric: "tabular-nums" }}
				>
					Window {formatDuration(editor.viewStartSec)} –{" "}
					{formatDuration(editor.viewStartSec + editor.viewWidthSec)}
				</Typography>
			</Box>
		</Box>
	);
}

function commitSegmentDrag(
	drag: SegmentDragState,
	apply: UseClipEditorReturn["apply"],
) {
	if (drag.mode === "move") {
		apply((edit) => {
			if (!edit.segments.some((s) => s.id === drag.segmentId)) return edit;
			const trackDelta = drag.ghostTrack - drag.originTrack;
			let next = edit;
			let maxTrack = edit.tracks - 1;
			for (const member of drag.group) {
				const ghost = drag.ghostStarts.find((g) => g.id === member.id);
				const start = ghost?.start ?? member.originStart;
				const track = Math.max(0, member.originTrack + trackDelta);
				next = moveSegment(next, member.id, start, track);
				maxTrack = Math.max(maxTrack, track);
			}
			return maxTrack >= edit.tracks ? { ...next, tracks: maxTrack + 1 } : next;
		});
		return;
	}
	if (drag.mode === "left") {
		apply((edit) => {
			if (!edit.segments.some((s) => s.id === drag.segmentId)) return edit;
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
		});
		return;
	}
	apply((edit) => {
		if (!edit.segments.some((s) => s.id === drag.segmentId)) return edit;
		return setSegmentRange(edit, drag.segmentId, drag.ghostIn, drag.ghostOut);
	});
}

function DragGhost(props: {
	leftFraction: number;
	widthFraction: number;
	label?: string;
	invalid?: boolean;
}) {
	return (
		<Box
			aria-hidden="true"
			style={{
				left: `${props.leftFraction}%`,
				width: `max(2px, ${props.widthFraction}%)`,
			}}
			sx={{
				position: "absolute",
				top: 8,
				bottom: 8,
				borderRadius: 1,
				border: "2px dashed",
				borderColor: props.invalid ? "error.main" : "primary.light",
				bgcolor: props.invalid
					? "rgba(248, 113, 113, 0.12)"
					: "rgba(56, 189, 248, 0.16)",
				pointerEvents: "none",
				zIndex: 5,
			}}
		>
			{props.label && (
				<Typography
					variant="caption"
					sx={{
						px: 0.5,
						whiteSpace: "nowrap",
						overflow: "hidden",
						textOverflow: "ellipsis",
						display: "block",
						lineHeight: 1.6,
					}}
				>
					{props.label}
				</Typography>
			)}
		</Box>
	);
}

function PhantomTrackRow(props: {
	label: string;
	ghosts: Array<{
		key: string;
		leftFraction: number;
		widthFraction: number;
		name?: string;
		invalid?: boolean;
	}>;
}) {
	return (
		<TimelineRow label={props.label}>
			<Box
				sx={{
					position: "relative",
					height: TRACK_HEIGHT_PX,
					mb: 0.5,
					borderRadius: 1,
					border: "2px dashed",
					borderColor: "primary.dark",
					overflow: "hidden",
				}}
			>
				{props.ghosts.map((ghost) => (
					<DragGhost
						key={ghost.key}
						leftFraction={ghost.leftFraction}
						widthFraction={ghost.widthFraction}
						label={ghost.name}
						invalid={ghost.invalid}
					/>
				))}
				<Typography
					variant="caption"
					color="text.secondary"
					sx={{ position: "absolute", top: 2, left: 4 }}
				>
					New track
				</Typography>
			</Box>
		</TimelineRow>
	);
}

function clipNameOfDragged(
	drag: SegmentDragState,
	editor: UseClipEditorReturn,
	clipName: (segment: TimelineSegment) => string,
): string {
	const segment = editor.edit.segments.find((s) => s.id === drag.segmentId);
	return segment ? clipName(segment) : "";
}

function FloatingDragChip(props: { name: string; x: number; y: number }) {
	return (
		<Box
			aria-hidden="true"
			sx={{
				position: "fixed",
				left: props.x,
				top: props.y,
				transform: "translate(-50%, 14px)",
				pointerEvents: "none",
				zIndex: 1400,
				maxWidth: 240,
				px: 1,
				py: 0.5,
				borderRadius: 1,
				border: "1px dashed",
				borderColor: "error.main",
				bgcolor: "rgba(248, 113, 113, 0.14)",
				backdropFilter: "blur(4px)",
				overflow: "hidden",
			}}
		>
			<Typography variant="caption" noWrap>
				{props.name || "Clip"} · release to cancel
			</Typography>
		</Box>
	);
}

function ClampedEdgeWarning(props: { x: number; y: number }) {
	return (
		<Box
			aria-hidden="true"
			sx={{
				position: "fixed",
				left: props.x,
				top: props.y,
				transform: "translate(-50%, 14px)",
				pointerEvents: "none",
				zIndex: 1400,
				maxWidth: 260,
				px: 1,
				py: 0.5,
				borderRadius: 1,
				border: "1px solid",
				borderColor: "warning.main",
				bgcolor: "rgba(245, 158, 11, 0.14)",
				backdropFilter: "blur(4px)",
			}}
		>
			<Typography variant="caption" noWrap>
				Edge meets the next clip
			</Typography>
		</Box>
	);
}

function TrackCollisionWarning(props: { x: number; y: number }) {
	return (
		<Box
			aria-hidden="true"
			sx={{
				position: "fixed",
				left: props.x,
				top: props.y,
				transform: "translate(-50%, 14px)",
				pointerEvents: "none",
				zIndex: 1400,
				maxWidth: 320,
				px: 1,
				py: 0.5,
				borderRadius: 1,
				border: "1px solid",
				borderColor: "error.main",
				bgcolor: "rgba(248, 113, 113, 0.14)",
				backdropFilter: "blur(4px)",
			}}
		>
			<Typography variant="caption" noWrap>
				Cannot move: segments would overlap on the same track
			</Typography>
		</Box>
	);
}

function TimelineRuler(props: {
	fraction: (sec: number) => number;
	positionSec: number;
	onScrub: (sec: number) => void;
	viewStartSec: number;
	viewWidthSec: number;
}) {
	const scrubRef = useRef<{
		startX: number;
		startSec: number;
		active: boolean;
	}>({
		startX: 0,
		startSec: 0,
		active: false,
	});

	const secAtClientX = (element: HTMLElement, clientX: number) => {
		const bounds = element.getBoundingClientRect();
		const f = Math.min(
			1,
			Math.max(0, (clientX - bounds.left) / Math.max(1, bounds.width)),
		);
		return props.viewStartSec + f * props.viewWidthSec;
	};

	return (
		<TimelineRow label="Timeline">
			<Box
				role="slider"
				aria-label="Timeline scrubber"
				aria-valuemin={0}
				aria-valuemax={props.viewStartSec + props.viewWidthSec}
				aria-valuenow={Math.round(props.viewStartSec)}
				tabIndex={0}
				onPointerDown={(event) => {
					if (event.button !== 0) return;
					event.preventDefault();
					event.currentTarget.setPointerCapture(event.pointerId);
					scrubRef.current = {
						startX: event.clientX,
						startSec: secAtClientX(event.currentTarget, event.clientX),
						active: true,
					};
					props.onScrub(scrubRef.current.startSec);
				}}
				onPointerMove={(event) => {
					if (!scrubRef.current.active) return;
					props.onScrub(secAtClientX(event.currentTarget, event.clientX));
				}}
				onPointerUp={(event) => {
					if (!scrubRef.current.active) return;
					scrubRef.current.active = false;
					props.onScrub(secAtClientX(event.currentTarget, event.clientX));
				}}
				onPointerCancel={() => {
					scrubRef.current.active = false;
				}}
				onLostPointerCapture={() => {
					scrubRef.current.active = false;
				}}
				onKeyDown={(event) => {
					if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
					event.preventDefault();
					const delta = event.shiftKey ? 1 : 0.1;
					props.onScrub(
						Math.max(
							0,
							props.positionSec + (event.key === "ArrowRight" ? delta : -delta),
						),
					);
				}}
				sx={{
					position: "relative",
					height: 32,
					touchAction: "none",
					userSelect: "none",
					cursor: "ew-resize",
					overflow: "hidden",
				}}
			>
				{TIMELINE_AXIS_FRACTIONS.map((fraction) => {
					const sec = props.viewStartSec + fraction * props.viewWidthSec;
					return (
						<Box
							key={fraction}
							sx={{
								position: "absolute",
								top: 0,
								bottom: 0,
								left: `${fraction * 100}%`,
								ml: gridLineOffset(fraction),
								borderLeft: "1px solid",
								borderColor: "divider",
							}}
						>
							<Typography
								variant="caption"
								color="text.secondary"
								sx={{
									display: "block",
									transform: axisLabelTransform(fraction),
									whiteSpace: "nowrap",
									pl: 0.5,
									fontVariantNumeric: "tabular-nums",
									lineHeight: 1.4,
								}}
							>
								{formatDuration(sec)}
							</Typography>
						</Box>
					);
				})}
				<TimelinePlayhead percent={props.fraction(props.positionSec)} />
			</Box>
		</TimelineRow>
	);
}

function TrackRow(props: {
	track: number;
	guildId: string;
	editor: UseClipEditorReturn;
	clipName: (segment: TimelineSegment) => string;
	fraction: (sec: number) => number;
	pxPerSec: number;
	preview: DragPreviewState | null;
	active: boolean;
	onActivate: () => void;
	dragGhosts: Array<{
		segmentId: string;
		leftFraction: number;
		widthFraction: number;
		name: string;
		invalid: boolean;
	}>;
	draggingSegmentIds: string[];
	onRowRef: (element: HTMLElement | null) => void;
	onBeginSegmentDrag: (drag: SegmentDragState) => void;
	onBeginMarquee: (
		event: ReactPointerEvent<HTMLElement>,
		track: number,
	) => void;
}) {
	const { editor, track } = props;
	const segments = editor.edit.segments.filter((s) => s.track === track);
	const showPreview = props.preview?.track === track;

	// Rows are made of individual segments plus one box per merged unit,
	// so a merged chain looks and behaves like a single clip.
	const membersByGroup = new Map<string, TimelineSegment[]>();
	for (const segment of segments) {
		if (!segment.mergeGroup) continue;
		const members = membersByGroup.get(segment.mergeGroup) ?? [];
		members.push(segment);
		membersByGroup.set(segment.mergeGroup, members);
	}
	type RowElement =
		| { kind: "segment"; segment: TimelineSegment }
		| { kind: "group"; members: TimelineSegment[] };
	const renderedGroups = new Set<string>();
	const elements: RowElement[] = [];
	for (const segment of segments) {
		if (!segment.mergeGroup) {
			elements.push({ kind: "segment", segment });
			continue;
		}
		if (renderedGroups.has(segment.mergeGroup)) continue;
		renderedGroups.add(segment.mergeGroup);
		elements.push({
			kind: "group",
			members: membersByGroup.get(segment.mergeGroup) ?? [segment],
		});
	}

	return (
		<TimelineRow label={`Track ${track + 1}`}>
			<Box
				ref={(element: HTMLDivElement | null) => props.onRowRef(element)}
				onClick={props.onActivate}
				onPointerDown={(event) => props.onBeginMarquee(event, track)}
				sx={{
					position: "relative",
					height: TRACK_HEIGHT_PX,
					mb: 0.5,
					borderRadius: 1,
					bgcolor: props.active
						? "rgba(56, 189, 248, 0.09)"
						: "rgba(148, 163, 184, 0.06)",
					outline: props.active ? "1px solid rgba(56, 189, 248, 0.45)" : "none",
					outlineOffset: 1,
					cursor: "pointer",
					overflow: "hidden",
					touchAction: "none",
				}}
			>
				{elements.map((element) => {
					if (element.kind === "group") {
						const first = element.members[0];
						if (!first) return null;
						const start = Math.min(
							...element.members.map((member) =>
								props.fraction(member.timelineStart),
							),
						);
						const end = Math.max(
							...element.members.map((member) =>
								props.fraction(segmentEnd(member)),
							),
						);
						const width = end - start;
						if (width <= 0) return null;
						return (
							<MergedUnitBox
								key={first.mergeGroup}
								members={element.members}
								first={first}
								editor={editor}
								name={props.clipName(first)}
								selected={element.members.some((member) =>
									editor.selectedSegmentIds.includes(member.id),
								)}
								dragging={element.members.some((member) =>
									props.draggingSegmentIds.includes(member.id),
								)}
								leftFraction={start}
								widthFraction={width}
								maxTrack={editor.edit.tracks - 1}
								onBeginDrag={props.onBeginSegmentDrag}
							/>
						);
					}
					const segment = element.segment;
					const start = props.fraction(segment.timelineStart);
					const width = Math.max(
						0,
						props.fraction(segmentEnd(segment)) - start,
					);
					if (width <= 0) return null;
					return (
						<TrackSegment
							key={segment.id}
							segment={segment}
							guildId={props.guildId}
							editor={editor}
							name={props.clipName(segment)}
							selected={editor.selectedSegmentIds.includes(segment.id)}
							copied={editor.copySourceId === segment.id}
							dragging={props.draggingSegmentIds.includes(segment.id)}
							leftFraction={start}
							widthFraction={width}
							maxSource={
								editor.sourceDuration(segment.sourceId) ?? segment.sourceOut
							}
							maxTrack={editor.edit.tracks - 1}
							onSelect={() => editor.select(segment.id)}
							onBeginDrag={props.onBeginSegmentDrag}
						/>
					);
				})}
				{showPreview && props.preview && (
					<DragGhost
						leftFraction={props.fraction(props.preview.startSec)}
						widthFraction={Math.max(
							0,
							props.fraction(props.preview.startSec + props.preview.lengthSec) -
								props.fraction(props.preview.startSec),
						)}
					/>
				)}
				{props.dragGhosts.map((dragGhost) => (
					<DragGhost
						key={dragGhost.segmentId}
						leftFraction={dragGhost.leftFraction}
						widthFraction={dragGhost.widthFraction}
						label={dragGhost.name}
						invalid={dragGhost.invalid}
					/>
				))}
				<TimelinePlayhead percent={props.fraction(editor.positionSec)} />
			</Box>
		</TimelineRow>
	);
}

function TrackSegment(props: {
	segment: TimelineSegment;
	guildId: string;
	editor: UseClipEditorReturn;
	name: string;
	selected: boolean;
	copied: boolean;
	dragging: boolean;
	leftFraction: number;
	widthFraction: number;
	maxSource: number;
	maxTrack: number;
	onSelect: () => void;
	onBeginDrag: (drag: SegmentDragState) => void;
}) {
	const { segment, editor } = props;
	const sourcePeaks = useClipWaveform(props.guildId, segment.sourceId);
	const waveform = useProcessedSegmentWaveform(
		props.guildId,
		segment,
		sourcePeaks,
	);
	const durationSec = props.maxSource > 0 ? props.maxSource : segment.sourceOut;

	const beginGesture = (
		event: ReactPointerEvent<HTMLElement>,
		mode: SegmentDragMode,
	) => {
		if (event.button !== 0) return;
		event.preventDefault();
		event.stopPropagation();
		// Capture so releasing outside the window still delivers the pointerup
		// (the drag hook listens on window, which misses it otherwise).
		try {
			event.currentTarget.setPointerCapture(event.pointerId);
		} catch {
			// Best-effort: the window listeners still cover the usual drag.
		}
		const toGrouped = (s: TimelineSegment): GroupedSegment => ({
			id: s.id,
			originStart: s.timelineStart,
			originTrack: s.track,
			duration: segmentDuration(s),
		});
		const findSegment = (id: string) =>
			editor.edit.segments.find((s) => s.id === id);
		// Ctrl/Cmd-click toggles the segment in the selection: grabbing an
		// unselected segment adds it and drags the new selection; grabbing a
		// selected one removes it and drags only that segment.
		const modifierClick = event.ctrlKey || event.metaKey;
		let group: GroupedSegment[];
		if (modifierClick) {
			const next = editor.toggleSelect(segment.id);
			group = next.includes(segment.id)
				? next
						.map((id) => findSegment(id))
						.filter((s): s is TimelineSegment => Boolean(s))
						.map(toGrouped)
				: [toGrouped(segment)];
		} else {
			// Grabbing a selected segment keeps the whole selection and moves
			// it as a group; grabbing anything else replaces the selection.
			// Edge trims only ever touch the grabbed segment.
			group = props.selected
				? editor.selectedSegments.map(toGrouped)
				: [toGrouped(segment)];
			if (!props.selected) props.onSelect();
		}
		props.onBeginDrag({
			mode,
			segmentId: segment.id,
			group,
			originStart: segment.timelineStart,
			originIn: segment.sourceIn,
			originOut: segment.sourceOut,
			originTrack: segment.track,
			originRate: effectiveRate(segment.effects),
			originTail: segment.effects.tailSeconds,
			reverse: segment.effects.reverse,
			maxSource: props.maxSource,
			maxTrack: props.maxTrack,
			startX: event.clientX,
			startY: event.clientY,
			ghostStart: segment.timelineStart,
			ghostIn: segment.sourceIn,
			ghostOut: segment.sourceOut,
			ghostTrack: segment.track,
			ghostStarts: [],
			modifierClick,
			trackCollision: false,
			valid: true,
			clamped: false,
			pointerX: event.clientX,
			pointerY: event.clientY,
		});
	};

	return (
		<Box
			onPointerDown={(event) => beginGesture(event, "move")}
			onDoubleClick={props.onSelect}
			sx={{
				position: "absolute",
				top: 8,
				bottom: 8,
				left: `${props.leftFraction}%`,
				width: `max(2px, ${props.widthFraction}%)`,
				minWidth: HANDLE_WIDTH_PX * 2,
				borderRadius: 1,
				opacity: props.dragging ? 0.45 : 1,
				bgcolor: props.selected
					? "rgba(168, 85, 247, 0.65)"
					: "rgba(56, 189, 248, 0.35)",
				border: props.selected ? "2px solid" : "1px solid",
				borderColor: props.selected
					? "secondary.main"
					: "rgba(56, 189, 248, 0.55)",
				boxShadow: props.selected
					? "0 0 0 3px rgba(217, 70, 239, 0.35), 0 2px 10px rgba(2, 6, 23, 0.6)"
					: "0 1px 3px rgba(2, 6, 23, 0.4)",
				cursor: "grab",
				userSelect: "none",
				overflow: "hidden",
				zIndex: props.selected ? 4 : 2,
			}}
		>
			<SegmentWaveform
				peaks={waveform.peaks}
				sourceIn={segment.sourceIn}
				sourceOut={segment.sourceOut}
				durationSec={durationSec}
				selected={props.selected}
				reverse={segment.effects.reverse}
				processed={waveform.processed}
			/>
			{props.copied && (
				<Box
					component="svg"
					aria-hidden="true"
					sx={{
						position: "absolute",
						inset: 0,
						width: "100%",
						height: "100%",
						pointerEvents: "none",
						zIndex: 5,
						animation: `${copiedDashes} 1s linear infinite`,
						"@media (prefers-reduced-motion: reduce)": {
							animation: "none",
						},
					}}
				>
					<rect
						x="2"
						y="2"
						width="calc(100% - 4px)"
						height="calc(100% - 4px)"
						rx="4"
						fill="none"
						stroke="rgba(125, 211, 252, 0.9)"
						strokeWidth="2"
						strokeDasharray="10 8"
					/>
				</Box>
			)}
			<Box
				onPointerDown={(event) => beginGesture(event, "left")}
				sx={{
					position: "absolute",
					top: 0,
					bottom: 0,
					left: 0,
					width: HANDLE_WIDTH_PX,
					cursor: "ew-resize",
				}}
			/>
			<Box
				onPointerDown={(event) => beginGesture(event, "right")}
				sx={{
					position: "absolute",
					top: 0,
					bottom: 0,
					right: 0,
					width: HANDLE_WIDTH_PX,
					cursor: "ew-resize",
				}}
			/>
			<Typography
				variant="caption"
				sx={{
					px: 0.5,
					whiteSpace: "nowrap",
					overflow: "hidden",
					textOverflow: "ellipsis",
					display: "block",
					lineHeight: 1.6,
					position: "relative",
					zIndex: 1,
					textShadow: "0 1px 3px rgba(2, 6, 23, 0.9)",
				}}
			>
				{props.name}
			</Typography>
		</Box>
	);
}

/**
 * One box for a merged unit: spans its whole chain and moves as a rigid
 * group. The member segments keep their own sources and effects, so the
 * unit has no trim handles - ungroup (or undo) to edit the pieces.
 */
function MergedUnitBox(props: {
	members: TimelineSegment[];
	first: TimelineSegment;
	editor: UseClipEditorReturn;
	name: string;
	selected: boolean;
	dragging: boolean;
	leftFraction: number;
	widthFraction: number;
	maxTrack: number;
	onBeginDrag: (drag: SegmentDragState) => void;
}) {
	const { members, editor } = props;

	const beginGesture = (event: ReactPointerEvent<HTMLElement>) => {
		if (event.button !== 0) return;
		event.preventDefault();
		event.stopPropagation();
		try {
			event.currentTarget.setPointerCapture(event.pointerId);
		} catch {
			// Best-effort: the window listeners still cover the usual drag.
		}
		const toGrouped = (s: TimelineSegment): GroupedSegment => ({
			id: s.id,
			originStart: s.timelineStart,
			originTrack: s.track,
			duration: segmentDuration(s),
		});
		const first = props.first;
		// Selecting every member turns the existing multi-selection drag
		// machinery into a rigid group move.
		editor.selectMany(members.map((member) => member.id));
		props.onBeginDrag({
			mode: "move",
			segmentId: first.id,
			group: members.map(toGrouped),
			originStart: first.timelineStart,
			originIn: first.sourceIn,
			originOut: first.sourceOut,
			originTrack: first.track,
			originRate: effectiveRate(first.effects),
			originTail: first.effects.tailSeconds,
			reverse: first.effects.reverse,
			maxSource: first.sourceOut,
			maxTrack: props.maxTrack,
			startX: event.clientX,
			startY: event.clientY,
			ghostStart: first.timelineStart,
			ghostIn: first.sourceIn,
			ghostOut: first.sourceOut,
			ghostTrack: first.track,
			ghostStarts: [],
			modifierClick: event.ctrlKey || event.metaKey,
			trackCollision: false,
			valid: true,
			clamped: false,
			pointerX: event.clientX,
			pointerY: event.clientY,
		});
	};

	return (
		<Box
			onPointerDown={beginGesture}
			onDoubleClick={() =>
				editor.selectMany(members.map((member) => member.id))
			}
			aria-label={`Merged unit of ${members.length} clips`}
			sx={{
				position: "absolute",
				top: 8,
				bottom: 8,
				left: `${props.leftFraction}%`,
				width: `max(2px, ${props.widthFraction}%)`,
				borderRadius: 1,
				opacity: props.dragging ? 0.45 : 1,
				bgcolor: props.selected
					? "rgba(168, 85, 247, 0.65)"
					: "rgba(45, 212, 191, 0.2)",
				border: props.selected ? "2px solid" : "1px dashed",
				borderColor: props.selected
					? "secondary.main"
					: "rgba(45, 212, 191, 0.55)",
				boxShadow: props.selected
					? "0 0 0 3px rgba(217, 70, 239, 0.35), 0 2px 10px rgba(2, 6, 23, 0.6)"
					: "0 1px 3px rgba(2, 6, 23, 0.4)",
				cursor: "grab",
				userSelect: "none",
				overflow: "hidden",
				zIndex: props.selected ? 4 : 2,
			}}
		>
			<Typography
				variant="caption"
				sx={{
					px: 0.5,
					whiteSpace: "nowrap",
					overflow: "hidden",
					textOverflow: "ellipsis",
					display: "block",
					lineHeight: 1.6,
					position: "relative",
					zIndex: 1,
					textShadow: "0 1px 3px rgba(2, 6, 23, 0.9)",
				}}
			>
				{props.name}
				{members.length > 1 ? ` +${members.length - 1}` : ""}
			</Typography>
			<Typography
				variant="caption"
				color="text.secondary"
				sx={{
					position: "absolute",
					top: 2,
					right: 4,
					fontSize: 10,
					lineHeight: 1.4,
					textShadow: "0 1px 3px rgba(2, 6, 23, 0.9)",
				}}
			>
				merged
			</Typography>
		</Box>
	);
}
