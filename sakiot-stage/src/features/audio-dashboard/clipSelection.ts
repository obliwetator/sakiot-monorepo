import type { SessionSelection } from "./logicalSessionSelection";

export type SelectionEdge = "start" | "end";

/** Widths the detail editor can zoom to, narrowest first. */
export const DETAIL_WINDOWS_MS = [
	5_000, 10_000, 30_000, 60_000, 300_000, 1_800_000,
] as const;
export const DEFAULT_DETAIL_WINDOW_MS = 60_000;

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

/** Steps to the next zoom preset; `direction` 1 zooms in, -1 zooms out. */
export function zoomDetailWindow(windowMs: number, direction: 1 | -1): number {
	const index = DETAIL_WINDOWS_MS.findIndex(
		(candidate) => candidate >= windowMs,
	);
	const current = index === -1 ? DETAIL_WINDOWS_MS.length - 1 : index;
	const next = Math.min(
		DETAIL_WINDOWS_MS.length - 1,
		Math.max(0, current - direction),
	);
	return DETAIL_WINDOWS_MS[next];
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

export function nudgeEdge(
	selection: SessionSelection,
	edge: SelectionEdge,
	deltaMs: number,
	durationMs: number,
): SessionSelection {
	const current = edge === "start" ? selection[0] : selection[1];
	return applyEdge(selection, edge, current + deltaMs, durationMs);
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

/**
 * Pointer precision while dragging: moving away from the handle vertically
 * scales the horizontal travel down, the way iOS scrubbing does.
 */
export function fineDragMultiplier(dyPx: number): number {
	const distance = Math.abs(dyPx);
	if (!Number.isFinite(distance) || distance < 24) return 1;
	if (distance < 96) return 10;
	return 100;
}

export interface FineDragState {
	/** Pointer x the last move was measured from. */
	lastPx: number;
	valueMs: number;
	multiplier: number;
}

export function beginFineDrag(xPx: number, valueMs: number): FineDragState {
	return { lastPx: xPx, valueMs, multiplier: 1 };
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
		state: { lastPx: pointer.xPx, valueMs, multiplier },
		valueMs,
	};
}
