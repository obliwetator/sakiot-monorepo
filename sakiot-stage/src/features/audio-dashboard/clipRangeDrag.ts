import {
	advanceFineDrag,
	applyEdgeWithinWindow,
	constrainFineDragToLens,
	type FineDragState,
	moveSelectionWithinWindow,
	type SelectionEdge,
	type TimeWindow,
} from "./clipSelection";
import type { SessionSelection } from "./logicalSessionSelection";

export const VIEW_DRAG_THRESHOLD_PX = 4;

export type DragKind = { type: "edge"; edge: SelectionEdge } | { type: "band" };
type DragFeedbackKind = DragKind | { type: "playhead" };

export interface SelectionDragState {
	kind: DragKind;
	startX: number;
	startY: number;
	plotLeftPx: number;
	plotWidthPx: number;
	fine: FineDragState;
	/** Selection the gesture started from; the commit compares against it. */
	origin: SessionSelection;
	/** Fixed boundary for a whole-band drag, captured when the pointer goes down. */
	movementWindow: TimeWindow;
	moved: boolean;
	/** Ghost selection following the pointer during the drag. */
	selection: SessionSelection;
	valid: boolean;
}

export function computeSelectionDrag(
	drag: SelectionDragState,
	pointer: { clientX: number; clientY: number },
	msPerPx: number,
	durationMs: number,
	plotBottomPx: number | null,
): SelectionDragState {
	if (msPerPx <= 0) return drag;
	if (
		drag.kind.type === "band" &&
		!drag.moved &&
		Math.hypot(pointer.clientX - drag.startX, pointer.clientY - drag.startY) <
			VIEW_DRAG_THRESHOLD_PX
	) {
		return drag;
	}
	const advanced = advanceFineDrag(drag.fine, {
		xPx: pointer.clientX,
		dyPx: pointer.clientY - drag.startY,
		msPerPx,
	});
	const constrained = constrainFineDragToLens(
		advanced.state,
		drag.movementWindow.endMs - drag.movementWindow.startMs,
		durationMs,
	);
	const nextSelection =
		drag.kind.type === "edge"
			? applyEdgeWithinWindow(
					drag.origin,
					drag.kind.edge,
					constrained.valueMs,
					durationMs,
					drag.movementWindow,
				)
			: moveSelectionWithinWindow(
					drag.origin,
					constrained.valueMs - drag.origin[0],
					drag.movementWindow,
				);
	// Do not accumulate invisible pointer travel after the band reaches a
	// boundary, so reversing direction moves it immediately.
	const fine =
		drag.kind.type === "band"
			? { ...constrained, valueMs: nextSelection[0] }
			: constrained;
	const valid =
		plotBottomPx === null ? true : pointer.clientY <= plotBottomPx + 32;
	return { ...drag, fine, selection: nextSelection, valid, moved: true };
}

export function selectionDragFeedback(
	drag: SelectionDragState,
	event: { clientX: number; clientY: number },
): DragFeedback {
	return {
		kind: drag.kind,
		multiplier: drag.fine.multiplier,
		valueMs: drag.fine.valueMs,
		originMs: drag.fine.originMs,
		lensAnchorMs: drag.fine.lensAnchorMs,
		dyPx: event.clientY - drag.startY,
		startYPx: drag.startY,
		pointerXPx: event.clientX,
		pointerYPx: event.clientY,
		plotLeftPx: drag.plotLeftPx,
		plotWidthPx: drag.plotWidthPx,
	};
}

export interface DragFeedback {
	kind: DragFeedbackKind;
	multiplier: number;
	valueMs: number;
	originMs: number;
	lensAnchorMs: number;
	dyPx: number;
	startYPx: number;
	pointerXPx: number;
	pointerYPx: number;
	plotLeftPx: number;
	plotWidthPx: number;
	/** Fixed movement range shared by ×10 and ×100 playhead modes. */
	limitWindow?: TimeWindow;
}

export type ViewDragKind = "overview" | "detail";

export interface ViewDragSession {
	kind: ViewDragKind;
	pointerId: number;
	startXPx: number;
	startYPx: number;
	pointerXPx: number;
	pointerYPx: number;
	leftPx: number;
	widthPx: number;
	msPerPx: number;
	origin: TimeWindow;
	originSelection: SessionSelection;
	fine?: FineDragState;
	fineRange?: TimeWindow;
	moved: boolean;
}

export function timeAtPointer(
	clientXPx: number,
	leftPx: number,
	widthPx: number,
	window: TimeWindow,
): number {
	const fraction = Math.min(
		1,
		Math.max(0, (clientXPx - leftPx) / Math.max(1, widthPx)),
	);
	return window.startMs + fraction * (window.endMs - window.startMs);
}
