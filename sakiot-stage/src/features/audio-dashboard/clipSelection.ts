import type { SessionSelection } from "./logicalSessionSelection";

export type SelectionEdge = "start" | "end";

/** Widths the detail editor can zoom to, narrowest first. */
export const DETAIL_WINDOWS_MS = [
	5_000, 10_000, 30_000, 60_000, 300_000, 1_800_000,
] as const;
export const DEFAULT_DETAIL_WINDOW_FRACTION = 0.1;

/** Keeps a dragged edge off the very edge of the view so you can see context. */
const PAN_MARGIN_FRACTION = 0.08;

export interface TimeWindow {
	startMs: number;
	endMs: number;
}

export function clampMs(valueMs: number, durationMs: number): number {
	if (!Number.isFinite(valueMs)) return 0;
	return Math.min(Math.max(valueMs, 0), Math.max(0, durationMs));
}

/** The initial detail viewport occupies ten percent of the whole session. */
export function defaultDetailWindowMs(durationMs: number): number {
	if (!Number.isFinite(durationMs)) return 0;
	return Math.max(0, durationMs) * DEFAULT_DETAIL_WINDOW_FRACTION;
}

/** Steps to the next zoom preset; `direction` 1 zooms in, -1 zooms out. */
export function zoomDetailWindow(
	windowMs: number,
	direction: 1 | -1,
	additionalStopMs?: number,
): number {
	const stops = Array.from(
		new Set([
			...DETAIL_WINDOWS_MS,
			...(additionalStopMs !== undefined && additionalStopMs > 0
				? [additionalStopMs]
				: []),
		]),
	).sort((left, right) => left - right);
	if (direction === 1) {
		return stops.findLast((candidate) => candidate < windowMs) ?? windowMs;
	}
	return stops.find((candidate) => candidate > windowMs) ?? windowMs;
}

function windowOfWidth(startMs: number, widthMs: number, durationMs: number) {
	const width = Math.min(widthMs, Math.max(durationMs, 1));
	const start = Math.min(Math.max(startMs, 0), Math.max(0, durationMs - width));
	return { startMs: start, endMs: start + width };
}

/** A window of `windowMs` centred on an instant, clamped to the recording. */
export function windowAround(
	centerMs: number,
	windowMs: number,
	durationMs: number,
): TimeWindow {
	return windowOfWidth(centerMs - windowMs / 2, windowMs, durationMs);
}

export function windowCenter(window: TimeWindow): number {
	return (window.startMs + window.endMs) / 2;
}

/** Slides a fixed-width detail window, stopping at either recording boundary. */
export function shiftWindow(
	window: TimeWindow,
	deltaMs: number,
	durationMs: number,
): TimeWindow {
	return windowOfWidth(
		window.startMs + deltaMs,
		window.endMs - window.startMs,
		durationMs,
	);
}

/**
 * Carries a selection through a viewport pan or zoom. The selection keeps its
 * fractional position in the viewport and scales with its width. If an older
 * independent viewport move left the selection outside, fit it back into the
 * source viewport first so the next interaction repairs the state.
 */
export function transformSelectionWithWindow(
	selection: SessionSelection,
	fromWindow: TimeWindow,
	toWindow: TimeWindow,
): SessionSelection {
	const fromStart = Math.min(fromWindow.startMs, fromWindow.endMs);
	const fromEnd = Math.max(fromWindow.startMs, fromWindow.endMs);
	const toStart = Math.min(toWindow.startMs, toWindow.endMs);
	const toEnd = Math.max(toWindow.startMs, toWindow.endMs);
	const fromWidth = fromEnd - fromStart;
	const toWidth = toEnd - toStart;
	if (fromWidth <= 0 || toWidth <= 0) return [toStart, toStart];

	const selectionWidth = Math.min(
		Math.max(0, selection[1] - selection[0]),
		fromWidth,
	);
	const fittedStart = Math.min(
		Math.max(selection[0], fromStart),
		fromEnd - selectionWidth,
	);
	const scale = toWidth / fromWidth;
	return [
		toStart + (fittedStart - fromStart) * scale,
		toStart + (fittedStart + selectionWidth - fromStart) * scale,
	];
}

export function selectionFitsWindow(
	selection: SessionSelection,
	windowMs: number,
): boolean {
	return selection[1] - selection[0] <= windowMs;
}

/** A window of `windowMs` centred on the selection, never narrower than it. */
export function windowForSelection(
	selection: SessionSelection,
	windowMs: number,
	durationMs: number,
): TimeWindow {
	const selectionMs = Math.max(0, selection[1] - selection[0]);
	const width = Math.max(windowMs, selectionMs);
	const center = (selection[0] + selection[1]) / 2;
	return windowOfWidth(center - width / 2, width, durationMs);
}

/**
 * The edge that changed between two selections, or null when neither did.
 * Drives which end of a long selection the view scrolls to.
 */
export function changedEdge(
	previous: SessionSelection,
	next: SessionSelection,
): SelectionEdge | null {
	if (next[0] !== previous[0]) return "start";
	if (next[1] !== previous[1]) return "end";
	return null;
}

/** True when both edges moved together without changing the selected length. */
export function selectionShiftedAsBand(
	previous: SessionSelection,
	next: SessionSelection,
): boolean {
	if (next[0] === previous[0] || next[1] === previous[1]) return false;
	const previousLength = previous[1] - previous[0];
	const nextLength = next[1] - next[0];
	return Math.abs(nextLength - previousLength) < 0.001;
}

/**
 * Shifts a window by the least amount that brings `valueMs` back inside it,
 * so dragging an edge past the view scrolls rather than losing the handle.
 */
export function panWindowToInclude(
	window: TimeWindow,
	valueMs: number,
	durationMs: number,
): TimeWindow {
	const width = window.endMs - window.startMs;
	if (width <= 0) return window;
	const margin = width * PAN_MARGIN_FRACTION;
	if (valueMs < window.startMs + margin) {
		return windowOfWidth(valueMs - margin, width, durationMs);
	}
	if (valueMs > window.endMs - margin) {
		return windowOfWidth(valueMs + margin - width, width, durationMs);
	}
	return windowOfWidth(window.startMs, width, durationMs);
}

/** Position of an instant inside a window, 0 at its start and 1 at its end. */
export function windowFraction(valueMs: number, window: TimeWindow): number {
	const width = window.endMs - window.startMs;
	if (width <= 0) return 0;
	return (valueMs - window.startMs) / width;
}

/**
 * A half-width viewport that rolls inside a fixed precision range. Wherever
 * possible, the current value appears directly beneath the pointer.
 */
export function rollingRulerWindow(
	allowedWindow: TimeWindow,
	valueMs: number,
	pointerFraction: number,
	visibleWidthMs = (allowedWindow.endMs - allowedWindow.startMs) / 2,
): TimeWindow {
	const allowedWidth = Math.max(0, allowedWindow.endMs - allowedWindow.startMs);
	if (allowedWidth <= 0) return allowedWindow;
	const width = Math.min(allowedWidth, Math.max(0, visibleWidthMs));
	if (width <= 0) return allowedWindow;
	const pointer = Math.min(1, Math.max(0, pointerFraction));
	const start = Math.min(
		Math.max(valueMs - pointer * width, allowedWindow.startMs),
		allowedWindow.endMs - width,
	);
	return { startMs: start, endMs: start + width };
}

/** Signed edge pressure: -1 at the left edge, +1 at the right, 0 centrally. */
export function rollingEdgeStrength(
	pointerXPx: number,
	leftPx: number,
	widthPx: number,
	edgeZonePx: number,
): number {
	const width = Math.max(1, widthPx);
	const zone = Math.min(Math.max(1, edgeZonePx), width / 2);
	const localX = pointerXPx - leftPx;
	if (localX < zone) return -Math.min(1, (zone - localX) / zone);
	if (localX > width - zone) {
		return Math.min(1, (localX - (width - zone)) / zone);
	}
	return 0;
}

export interface SelectionWindowGeometry {
	startFraction: number;
	endFraction: number;
	startHandleVisible: boolean;
	endHandleVisible: boolean;
	overlaps: boolean;
}

/**
 * Clips a selection band to the visible window without pretending that an
 * off-screen endpoint is a draggable handle parked on the window boundary.
 */
export function selectionWindowGeometry(
	selection: SessionSelection,
	window: TimeWindow,
): SelectionWindowGeometry {
	const rawStart = windowFraction(selection[0], window);
	const rawEnd = windowFraction(selection[1], window);
	return {
		startFraction: Math.min(1, Math.max(0, rawStart)),
		endFraction: Math.min(1, Math.max(0, rawEnd)),
		startHandleVisible: rawStart >= 0 && rawStart <= 1,
		endHandleVisible: rawEnd >= 0 && rawEnd <= 1,
		overlaps: selection[1] >= window.startMs && selection[0] <= window.endMs,
	};
}

export function windowMsPerPixel(window: TimeWindow, widthPx: number): number {
	if (widthPx <= 0) return 0;
	return (window.endMs - window.startMs) / widthPx;
}

/**
 * Moves one edge, keeping the pair ordered and inside the recording. Clip
 * length limits are deliberately not enforced: the same selection drives
 * download and silence removal, which may legitimately span hours.
 */
export function applyEdge(
	selection: SessionSelection,
	edge: SelectionEdge,
	valueMs: number,
	durationMs: number,
): SessionSelection {
	const value = clampMs(valueMs, durationMs);
	if (edge === "start") return [Math.min(value, selection[1]), selection[1]];
	return [selection[0], Math.max(value, selection[0])];
}

/** Moves one edge without allowing it to leave the visible clip window. */
export function applyEdgeWithinWindow(
	selection: SessionSelection,
	edge: SelectionEdge,
	valueMs: number,
	durationMs: number,
	window: TimeWindow,
): SessionSelection {
	const startLimit = Math.min(window.startMs, window.endMs);
	const endLimit = Math.max(window.startMs, window.endMs);
	const value = Math.min(Math.max(valueMs, startLimit), endLimit);
	return applyEdge(selection, edge, value, durationMs);
}

export function nudgeEdge(
	selection: SessionSelection,
	edge: SelectionEdge,
	deltaMs: number,
	durationMs: number,
): SessionSelection {
	const current = edge === "start" ? selection[0] : selection[1];
	return applyEdge(selection, edge, current + deltaMs, durationMs);
}

/** Edge the playhead is visually closest to, with the midpoint favouring In. */
export function nearestSelectionEdge(
	selection: SessionSelection,
	positionMs: number,
): SelectionEdge {
	return positionMs <= (selection[0] + selection[1]) / 2 ? "start" : "end";
}

/** Prevents an explicit In/Out action from collapsing or crossing the range. */
export function canSetSelectionEdge(
	selection: SessionSelection,
	edge: SelectionEdge,
	positionMs: number,
): boolean {
	return edge === "start"
		? positionMs < selection[1]
		: positionMs > selection[0];
}

/** Moves whichever edge is closest to the chosen position. */
export function setNearestSelectionEdge(
	selection: SessionSelection,
	positionMs: number,
	durationMs: number,
): SessionSelection {
	return applyEdge(
		selection,
		nearestSelectionEdge(selection, positionMs),
		positionMs,
		durationMs,
	);
}

/** Slides the whole selection, preserving its length at either boundary. */
export function moveSelection(
	selection: SessionSelection,
	deltaMs: number,
	durationMs: number,
): SessionSelection {
	const length = selection[1] - selection[0];
	const limit = Math.max(0, durationMs - length);
	const start = Math.min(Math.max(selection[0] + deltaMs, 0), limit);
	return [start, start + length];
}

/** Slides a selection without allowing either edge to leave the detail view. */
export function moveSelectionWithinWindow(
	selection: SessionSelection,
	deltaMs: number,
	window: TimeWindow,
): SessionSelection {
	const startLimit = Math.min(window.startMs, window.endMs);
	const endLimit = Math.max(window.startMs, window.endMs);
	const length = Math.max(0, selection[1] - selection[0]);
	if (length > endLimit - startLimit) return selection;
	const start = Math.min(
		Math.max(selection[0] + deltaMs, startLimit),
		endLimit - length,
	);
	return [start, start + length];
}

export const FINE_DRAG_START_PX = 32;
export const ULTRA_FINE_DRAG_START_PX = 160;

export interface PrecisionZoneBounds {
	topPx: number;
	bottomPx: number;
}

/** Screen-space band occupied by the active vertical precision level. */
export function precisionZoneBounds(
	dragStartYPx: number,
	multiplier: number,
	viewportHeightPx: number,
): PrecisionZoneBounds {
	const viewportHeight = Math.max(0, viewportHeightPx);
	const fineBoundary = Math.min(
		viewportHeight,
		Math.max(0, dragStartYPx - FINE_DRAG_START_PX),
	);
	const ultraBoundary = Math.min(
		viewportHeight,
		Math.max(0, dragStartYPx - ULTRA_FINE_DRAG_START_PX),
	);
	if (multiplier >= 100) return { topPx: 0, bottomPx: ultraBoundary };
	if (multiplier >= 10) {
		return { topPx: ultraBoundary, bottomPx: fineBoundary };
	}
	return { topPx: fineBoundary, bottomPx: viewportHeight };
}

/** Pulling upward from a handle reduces horizontal travel in two wide zones. */
export function fineDragMultiplier(dyPx: number): number {
	if (!Number.isFinite(dyPx)) return 1;
	const upwardDistance = Math.max(0, -dyPx);
	if (upwardDistance < FINE_DRAG_START_PX) return 1;
	if (upwardDistance < ULTRA_FINE_DRAG_START_PX) return 10;
	return 100;
}

/** Gives the precision lens enough surrounding context to make drift visible. */
export function precisionLensWindowMs(
	baseWindowMs: number,
	multiplier: number,
): number {
	if (multiplier >= 100) return Math.max(3_000, baseWindowMs / multiplier);
	return Math.max(10_000, baseWindowMs / Math.max(1, multiplier));
}

export interface FineDragState {
	/** Pointer x the last move was measured from. */
	lastPx: number;
	/** Original grabbed value; unchanged when precision levels change. */
	originMs: number;
	valueMs: number;
	multiplier: number;
	/** Fixed centre of the lens until the pointer enters another precision zone. */
	lensAnchorMs: number;
}

/** Keeps a fine-drag value inside the currently visible editor window. */
export function constrainFineDragToWindow(
	state: FineDragState,
	window: TimeWindow,
): FineDragState {
	return {
		...state,
		valueMs: Math.min(Math.max(state.valueMs, window.startMs), window.endMs),
	};
}

export function beginFineDrag(xPx: number, valueMs: number): FineDragState {
	return {
		lastPx: xPx,
		originMs: valueMs,
		valueMs,
		multiplier: 1,
		lensAnchorMs: valueMs,
	};
}

/** Stops an unnoticed long drift once the marker reaches a lens fade. */
export function constrainFineDragToLens(
	state: FineDragState,
	baseWindowMs: number,
	durationMs: number,
): FineDragState {
	if (state.multiplier <= 1) return state;
	const lens = windowAround(
		state.lensAnchorMs,
		precisionLensWindowMs(baseWindowMs, state.multiplier),
		durationMs,
	);
	return {
		...state,
		valueMs: Math.min(Math.max(state.valueMs, lens.startMs), lens.endMs),
	};
}

/**
 * Advances a drag by the travel since the previous move, at the precision the
 * pointer is currently at. Accumulating per move rather than from the grab
 * point means crossing a threshold changes the rate from that instant on,
 * without retroactively rescaling travel or jumping the selection.
 */
export function advanceFineDrag(
	state: FineDragState,
	pointer: { xPx: number; dyPx: number; msPerPx: number },
): { state: FineDragState; valueMs: number } {
	const multiplier = fineDragMultiplier(pointer.dyPx);
	const valueMs =
		state.valueMs +
		((pointer.xPx - state.lastPx) * pointer.msPerPx) / multiplier;
	return {
		state: {
			lastPx: pointer.xPx,
			originMs: state.originMs,
			valueMs,
			multiplier,
			lensAnchorMs:
				multiplier === state.multiplier ? state.lensAnchorMs : valueMs,
		},
		valueMs,
	};
}
