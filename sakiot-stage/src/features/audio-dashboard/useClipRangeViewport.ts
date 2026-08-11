import {
	type KeyboardEvent as ReactKeyboardEvent,
	type PointerEvent as ReactPointerEvent,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import { usePointerDrag } from "../../shared/pointerDrag";
import {
	computeSelectionDrag,
	type DragFeedback,
	type DragKind,
	type SelectionDragState,
	selectionDragFeedback,
	timeAtPointer,
	VIEW_DRAG_THRESHOLD_PX,
	type ViewDragKind,
	type ViewDragSession,
} from "./clipRangeDrag";
import type { ClipRangeEditorProps } from "./clipRangeEditorTypes";
import {
	advanceFineDrag,
	beginFineDrag,
	canSetSelectionEdge,
	changedEdge,
	constrainFineDragToWindow,
	defaultDetailWindowMs,
	FINE_DRAG_START_PX,
	nearestSelectionEdge,
	nudgeEdge,
	panWindowToInclude,
	precisionLensWindowMs,
	precisionZoneBounds,
	rollingEdgeStrength,
	rollingRulerWindow,
	type SelectionEdge,
	selectionFitsWindow,
	selectionShiftedAsBand,
	selectionWindowGeometry,
	setNearestSelectionEdge,
	shiftWindow,
	type TimeWindow,
	transformSelectionWithWindow,
	ULTRA_FINE_DRAG_START_PX,
	windowAround,
	windowCenter,
	windowForSelection,
	windowFraction,
	windowMsPerPixel,
	zoomDetailWindow,
} from "./clipSelection";
import {
	isValidClipSelection,
	type SessionSelection,
} from "./logicalSessionSelection";
import { useSessionWaveformPeaks } from "./WaveformCanvas";

const KEY_NUDGE_MS = 100;
const KEY_NUDGE_COARSE_MS = 1_000;
const PRECISE_AXIS_WINDOW_MS = 120_000;
const ROLLING_EDGE_ZONE_PX = 48;
const ROLLING_EDGE_MAX_PX_PER_SECOND = 240;

export function useClipRangeViewport(props: ClipRangeEditorProps) {
	const { durationMs, onSelectionChange, selection } = props;
	const defaultWindowMs = defaultDetailWindowMs(durationMs);
	const plotRef = useRef<HTMLDivElement | null>(null);
	const draggedSelectionRef = useRef<SessionSelection | null>(null);
	const viewDragRef = useRef<ViewDragSession | null>(null);
	const edgeDriveFrameRef = useRef<number | null>(null);
	const edgeDriveLastTimeRef = useRef<number | null>(null);
	const edgeDriveTickRef = useRef<(timestampMs: number) => void>(() => {});
	const [plotWidth, setPlotWidth] = useState(0);
	const [windowMs, setWindowMs] = useState(defaultWindowMs);
	const [dragFeedback, setDragFeedback] = useState<DragFeedback | null>(null);
	const [viewDragging, setViewDragging] = useState<ViewDragKind | null>(null);
	const [view, setView] = useState<TimeWindow>(() =>
		// Stamp focus wins over selection state because the draft is installed in
		// an effect immediately after this component's first render.
		props.initialFocusMs !== undefined
			? windowAround(props.initialFocusMs, defaultWindowMs, durationMs)
			: selectionFitsWindow(selection, defaultWindowMs)
				? windowForSelection(selection, defaultWindowMs, durationMs)
				: // A fresh session is selected end to end, which says nothing about
					// where a clip will be, so start the view on the playhead instead.
					windowAround(props.positionMs, defaultWindowMs, durationMs),
	);
	const msPerPx = windowMsPerPixel(view, plotWidth);
	const selectionDrag = usePointerDrag<SelectionDragState>({
		compute: (ghost, event) => {
			const next = computeSelectionDrag(
				ghost,
				event,
				msPerPx,
				durationMs,
				plotRef.current?.getBoundingClientRect().bottom ?? null,
			);
			setDragFeedback(selectionDragFeedback(next, event));
			return next;
		},
		onCommit: ({ ghost, pointerX }) => {
			setDragFeedback(null);
			commitSelectionDrag(ghost, pointerX);
		},
		onCancel: () => {
			setDragFeedback(null);
		},
	});
	const commitSelectionDrag = (drag: SelectionDragState, pointerX: number) => {
		if (drag.kind.type === "band" && !drag.moved) {
			const positionMs = timeAtPointer(
				pointerX,
				drag.plotLeftPx,
				drag.plotWidthPx,
				view,
			);
			props.onSeek(positionMs);
			onSelectionChange(
				setNearestSelectionEdge(drag.origin, positionMs, durationMs),
			);
			return;
		}
		if (
			drag.moved &&
			drag.valid &&
			(drag.selection[0] !== drag.origin[0] ||
				drag.selection[1] !== drag.origin[1])
		) {
			onSelectionChange(drag.selection);
		}
	};
	const previousSelectionRef = useRef(selection);
	const selectionRef = useRef(selection);
	useEffect(() => {
		selectionRef.current = selection;
	}, [selection]);
	// Read when the view is recomputed, but never a reason to recompute it:
	// the view must not chase the playhead frame by frame during playback.
	const positionRef = useRef(props.positionMs);
	useEffect(() => {
		positionRef.current = props.positionMs;
	}, [props.positionMs]);

	useEffect(() => {
		const plot = plotRef.current;
		if (!plot) return;
		setPlotWidth(plot.clientWidth);
		const observer = new ResizeObserver((entries) => {
			setPlotWidth(entries[0]?.contentRect.width ?? 0);
		});
		observer.observe(plot);
		return () => observer.disconnect();
	}, []);

	// The view chases whichever edge just moved, wherever it moved from — an in
	// point set by the I key hours away lands the same as one nudged by 0.1s.
	// Zooming keeps the centre, so the moment you were looking at stays put.
	useEffect(() => {
		const previous = previousSelectionRef.current;
		previousSelectionRef.current = selection;
		const moved = changedEdge(previous, selection);
		const movedByBand = selectionShiftedAsBand(previous, selection);
		const draggedSelection = draggedSelectionRef.current;
		const movedByDirectDrag = Boolean(
			draggedSelection &&
				selection[0] === draggedSelection[0] &&
				selection[1] === draggedSelection[1],
		);
		if (movedByDirectDrag) draggedSelectionRef.current = null;
		setView((current) => {
			const width = Math.min(windowMs, Math.max(durationMs, 1));
			if (Math.abs(current.endMs - current.startMs - width) > 0.5) {
				return windowAround(windowCenter(current), width, durationMs);
			}
			// A selection covering the whole recording is a reset, not an edit —
			// it is what a freshly loaded session starts on — so it parks the view
			// on the playhead rather than chasing an edge nobody placed.
			if (selection[0] <= 0 && selection[1] >= durationMs) {
				return windowAround(positionRef.current, width, durationMs);
			}
			// Whole-band drags stop at the visible boundaries; they must not make the
			// detail viewport chase the selection while it is being moved.
			if (movedByBand || movedByDirectDrag) return current;
			if (moved === null) return current;
			return panWindowToInclude(
				current,
				moved === "start" ? selection[0] : selection[1],
				durationMs,
			);
		});
	}, [durationMs, selection, windowMs]);

	const synchronizeSelectionWithView = useCallback(
		(
			sourceSelection: SessionSelection,
			sourceView: TimeWindow,
			nextView: TimeWindow,
		) => {
			const nextSelection = transformSelectionWithWindow(
				sourceSelection,
				sourceView,
				nextView,
			);
			const current = selectionRef.current;
			if (nextSelection[0] !== current[0] || nextSelection[1] !== current[1]) {
				selectionRef.current = nextSelection;
				draggedSelectionRef.current = nextSelection;
				onSelectionChange(nextSelection);
			}
			return nextSelection;
		},
		[onSelectionChange],
	);

	const zoom = useCallback(
		(direction: 1 | -1) => {
			const nextWindowMs = zoomDetailWindow(
				windowMs,
				direction,
				defaultWindowMs,
			);
			if (nextWindowMs === windowMs) return;
			const nextView = windowAround(
				windowCenter(view),
				Math.min(nextWindowMs, Math.max(durationMs, 1)),
				durationMs,
			);
			synchronizeSelectionWithView(selectionRef.current, view, nextView);
			setWindowMs(nextWindowMs);
			setView(nextView);
		},
		[defaultWindowMs, durationMs, synchronizeSelectionWithView, view, windowMs],
	);

	// Ctrl/⌘ so an ordinary scroll past a widget in the middle of a long page
	// still scrolls the page. Registered by hand because React attaches wheel
	// listeners passively, where preventDefault does nothing.
	useEffect(() => {
		const plot = plotRef.current;
		if (!plot) return;
		const onWheel = (event: WheelEvent) => {
			if (event.deltaY === 0 || !(event.ctrlKey || event.metaKey)) return;
			event.preventDefault();
			zoom(event.deltaY < 0 ? 1 : -1);
		};
		plot.addEventListener("wheel", onWheel, { passive: false });
		return () => plot.removeEventListener("wheel", onWheel);
	}, [zoom]);

	const stopEdgeDrive = useCallback(() => {
		if (edgeDriveFrameRef.current !== null) {
			cancelAnimationFrame(edgeDriveFrameRef.current);
			edgeDriveFrameRef.current = null;
		}
		edgeDriveLastTimeRef.current = null;
	}, []);

	useEffect(() => stopEdgeDrive, [stopEdgeDrive]);

	const endViewDrag = useCallback(() => {
		stopEdgeDrive();
		if (viewDragRef.current?.kind === "detail") {
			props.onSeekPreview?.(null);
			setDragFeedback(null);
		}
		viewDragRef.current = null;
		setViewDragging(null);
	}, [props.onSeekPreview, stopEdgeDrive]);

	// The precision zones use viewport coordinates captured when the drag starts.
	// If any scroll container moves, finish the gesture so the guide cannot float
	// over content that is no longer underneath its handle.
	useEffect(() => {
		const endDragOnScroll = () => {
			selectionDrag.cancel();
			if (viewDragRef.current) endViewDrag();
		};
		globalThis.addEventListener("scroll", endDragOnScroll, {
			capture: true,
			passive: true,
		});
		return () =>
			globalThis.removeEventListener("scroll", endDragOnScroll, {
				capture: true,
			});
	}, [selectionDrag.cancel, endViewDrag]);

	useEffect(() => {
		const cancelOnEscape = (event: KeyboardEvent) => {
			if (event.key !== "Escape") return;
			selectionDrag.cancel();
			if (viewDragRef.current) endViewDrag();
		};
		globalThis.addEventListener("keydown", cancelOnEscape);
		return () => globalThis.removeEventListener("keydown", cancelOnEscape);
	}, [selectionDrag.cancel, endViewDrag]);

	const { peaks } = useSessionWaveformPeaks(
		props.sessionId,
		props.silenceFree ?? false,
	);
	const displaySelection = selectionDrag.snapshot?.ghost.selection ?? selection;
	const selectionMs = displaySelection[1] - displaySelection[0];
	const valid = isValidClipSelection(displaySelection);
	const dragInvalid = selectionDrag.snapshot
		? !selectionDrag.snapshot.ghost.valid
		: false;
	const suggestedEdge = nearestSelectionEdge(
		displaySelection,
		props.positionMs,
	);
	const canSetStart = canSetSelectionEdge(
		displaySelection,
		"start",
		props.positionMs,
	);
	const canSetEnd = canSetSelectionEdge(
		displaySelection,
		"end",
		props.positionMs,
	);
	const startFraction = windowFraction(displaySelection[0], view);
	const endFraction = windowFraction(displaySelection[1], view);
	const selectionGeometry = selectionWindowGeometry(displaySelection, view);
	const playheadFraction = windowFraction(props.positionMs, view);
	const stampFraction =
		props.initialFocusMs === undefined
			? null
			: windowFraction(props.initialFocusMs, view);
	const preciseAxis = view.endMs - view.startMs <= PRECISE_AXIS_WINDOW_MS;
	const unconstrainedFineLimitWindow =
		dragFeedback && dragFeedback.multiplier > 1
			? windowAround(
					dragFeedback.lensAnchorMs,
					precisionLensWindowMs(
						view.endMs - view.startMs,
						dragFeedback.multiplier,
					),
					durationMs,
				)
			: null;
	const fineLimitWindow = dragFeedback?.limitWindow
		? dragFeedback.limitWindow
		: unconstrainedFineLimitWindow && dragFeedback?.kind.type === "playhead"
			? {
					startMs: Math.max(unconstrainedFineLimitWindow.startMs, view.startMs),
					endMs: Math.min(unconstrainedFineLimitWindow.endMs, view.endMs),
				}
			: unconstrainedFineLimitWindow;
	const fineWindow =
		fineLimitWindow && dragFeedback?.kind.type === "playhead"
			? rollingRulerWindow(
					fineLimitWindow,
					dragFeedback.valueMs,
					(dragFeedback.pointerXPx - dragFeedback.plotLeftPx) /
						dragFeedback.plotWidthPx,
					precisionLensWindowMs(
						view.endMs - view.startMs,
						dragFeedback.multiplier,
					) / 2,
				)
			: fineLimitWindow;
	const fineValueFraction =
		fineWindow && dragFeedback
			? windowFraction(dragFeedback.valueMs, fineWindow)
			: null;
	const fineLimitFraction =
		fineLimitWindow && dragFeedback
			? windowFraction(dragFeedback.valueMs, fineLimitWindow)
			: null;
	const rollingStrength =
		dragFeedback?.kind.type === "playhead" && dragFeedback.multiplier > 1
			? rollingEdgeStrength(
					dragFeedback.pointerXPx,
					dragFeedback.plotLeftPx,
					dragFeedback.plotWidthPx,
					ROLLING_EDGE_ZONE_PX,
				)
			: 0;
	const fineSelectionGeometry = fineWindow
		? selectionWindowGeometry(displaySelection, fineWindow)
		: null;
	const otherEdgeMs =
		dragFeedback?.kind.type === "edge"
			? displaySelection[dragFeedback.kind.edge === "start" ? 1 : 0]
			: null;
	const otherEdgeFraction =
		fineWindow && otherEdgeMs !== null
			? windowFraction(otherEdgeMs, fineWindow)
			: null;
	const precisionZone = dragFeedback
		? precisionZoneBounds(
				dragFeedback.startYPx,
				dragFeedback.multiplier,
				globalThis.innerHeight,
			)
		: null;
	const precisionBoundaries = dragFeedback
		? dragFeedback.multiplier >= 100
			? [
					{
						yPx: dragFeedback.startYPx - ULTRA_FINE_DRAG_START_PX,
						label: "Ultra ×100",
					},
				]
			: dragFeedback.multiplier >= 10
				? [
						{
							yPx: dragFeedback.startYPx - ULTRA_FINE_DRAG_START_PX,
							label: "Ultra ×100",
						},
					]
				: [
						{
							yPx: dragFeedback.startYPx - FINE_DRAG_START_PX,
							label: "Fine ×10",
						},
					]
		: [];

	const beginDrag = (event: ReactPointerEvent<HTMLElement>, kind: DragKind) => {
		if (event.button !== 0) return;
		event.preventDefault();
		event.stopPropagation();
		const plotBounds = plotRef.current?.getBoundingClientRect();
		const anchorMs =
			kind.type === "band"
				? selection[0]
				: selection[kind.edge === "start" ? 0 : 1];
		const drag: SelectionDragState = {
			kind,
			startX: event.clientX,
			startY: event.clientY,
			plotLeftPx: plotBounds?.left ?? 0,
			plotWidthPx: plotBounds?.width ?? plotWidth,
			fine: beginFineDrag(event.clientX, anchorMs),
			origin: selection,
			movementWindow: view,
			moved: false,
			selection,
			valid: true,
		};
		selectionDrag.begin(drag, event.clientX, event.clientY);
		setDragFeedback(selectionDragFeedback(drag, event));
	};

	const onHandleKeyDown = (
		event: ReactKeyboardEvent<HTMLElement>,
		edge: SelectionEdge,
	) => {
		if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
		event.preventDefault();
		event.stopPropagation();
		const distance = event.shiftKey ? KEY_NUDGE_COARSE_MS : KEY_NUDGE_MS;
		onSelectionChange(
			nudgeEdge(
				selection,
				edge,
				event.key === "ArrowRight" ? distance : -distance,
				durationMs,
			),
		);
	};

	const dragHandlers = (kind: DragKind) => ({
		onPointerDown: (event: ReactPointerEvent<HTMLElement>) =>
			beginDrag(event, kind),
	});

	const startEdgeDrive = () => {
		if (edgeDriveFrameRef.current !== null) return;
		edgeDriveLastTimeRef.current = null;
		edgeDriveFrameRef.current = requestAnimationFrame((timestampMs) =>
			edgeDriveTickRef.current(timestampMs),
		);
	};

	edgeDriveTickRef.current = (timestampMs) => {
		edgeDriveFrameRef.current = null;
		const drag = viewDragRef.current;
		const fine = drag?.fine;
		if (!drag || drag.kind !== "detail" || !fine || fine.multiplier <= 1) {
			edgeDriveLastTimeRef.current = null;
			return;
		}
		const strength = rollingEdgeStrength(
			drag.pointerXPx,
			drag.leftPx,
			drag.widthPx,
			ROLLING_EDGE_ZONE_PX,
		);
		if (strength === 0) {
			edgeDriveLastTimeRef.current = null;
			return;
		}
		const previousTimeMs = edgeDriveLastTimeRef.current ?? timestampMs;
		const elapsedSeconds =
			Math.min(50, Math.max(0, timestampMs - previousTimeMs)) / 1_000;
		edgeDriveLastTimeRef.current = timestampMs;
		const next = constrainFineDragToWindow(
			{
				...fine,
				valueMs:
					fine.valueMs +
					(strength *
						ROLLING_EDGE_MAX_PX_PER_SECOND *
						drag.msPerPx *
						elapsedSeconds) /
						fine.multiplier,
			},
			drag.fineRange ?? drag.origin,
		);
		if (next.valueMs === fine.valueMs && elapsedSeconds > 0) {
			edgeDriveLastTimeRef.current = null;
			return;
		}
		drag.fine = next;
		setDragFeedback({
			kind: { type: "playhead" },
			multiplier: next.multiplier,
			valueMs: next.valueMs,
			originMs: next.originMs,
			lensAnchorMs: next.lensAnchorMs,
			dyPx: drag.pointerYPx - drag.startYPx,
			startYPx: drag.startYPx,
			pointerXPx: drag.pointerXPx,
			pointerYPx: drag.pointerYPx,
			plotLeftPx: drag.leftPx,
			plotWidthPx: drag.widthPx,
			limitWindow: drag.fineRange,
		});
		if (props.onSeekPreview) props.onSeekPreview(next.valueMs);
		else props.onSeek(next.valueMs);
		edgeDriveFrameRef.current = requestAnimationFrame((nextTimestampMs) =>
			edgeDriveTickRef.current(nextTimestampMs),
		);
	};

	const beginViewDrag = (
		event: ReactPointerEvent<HTMLElement>,
		kind: ViewDragKind,
	) => {
		if (event.button !== 0) return;
		event.preventDefault();
		event.stopPropagation();
		event.currentTarget.setPointerCapture(event.pointerId);
		const bounds =
			(kind === "detail"
				? plotRef.current
				: event.currentTarget
			)?.getBoundingClientRect() ?? event.currentTarget.getBoundingClientRect();
		const widthPx = Math.max(1, bounds.width);
		let origin = view;
		let originSelection = selectionRef.current;
		if (kind === "overview") {
			const pointerMs = ((event.clientX - bounds.left) / widthPx) * durationMs;
			if (pointerMs < view.startMs || pointerMs > view.endMs) {
				origin = windowAround(pointerMs, view.endMs - view.startMs, durationMs);
				originSelection = synchronizeSelectionWithView(
					originSelection,
					view,
					origin,
				);
				setView(origin);
			}
		}
		const drag: ViewDragSession = {
			kind,
			pointerId: event.pointerId,
			startXPx: event.clientX,
			startYPx: event.clientY,
			pointerXPx: event.clientX,
			pointerYPx: event.clientY,
			leftPx: bounds.left,
			widthPx,
			msPerPx:
				kind === "overview"
					? durationMs / widthPx
					: (view.endMs - view.startMs) / widthPx,
			origin,
			originSelection,
			fine:
				kind === "detail"
					? beginFineDrag(
							event.clientX,
							timeAtPointer(event.clientX, bounds.left, widthPx, origin),
						)
					: undefined,
			moved: false,
		};
		viewDragRef.current = drag;
		setViewDragging(kind);
		if (kind === "detail") {
			const positionMs = drag.fine?.valueMs ?? 0;
			setDragFeedback({
				kind: { type: "playhead" },
				multiplier: 1,
				valueMs: positionMs,
				originMs: positionMs,
				lensAnchorMs: positionMs,
				dyPx: 0,
				startYPx: event.clientY,
				pointerXPx: event.clientX,
				pointerYPx: event.clientY,
				plotLeftPx: bounds.left,
				plotWidthPx: widthPx,
			});
			if (props.onSeekPreview) props.onSeekPreview(positionMs);
			else props.onSeek(positionMs);
		}
	};

	const continueViewDrag = (event: ReactPointerEvent<HTMLElement>) => {
		const drag = viewDragRef.current;
		if (!drag || drag.pointerId !== event.pointerId) return;
		event.stopPropagation();
		const deltaPx = event.clientX - drag.startXPx;
		if (drag.kind === "detail") {
			if (!drag.fine) return;
			stopEdgeDrive();
			drag.pointerXPx = event.clientX;
			drag.pointerYPx = event.clientY;
			drag.moved = drag.moved || Math.abs(deltaPx) >= VIEW_DRAG_THRESHOLD_PX;
			const advanced = advanceFineDrag(drag.fine, {
				xPx: event.clientX,
				dyPx: event.clientY - drag.startYPx,
				msPerPx: drag.msPerPx,
			});
			if (advanced.state.multiplier <= 1) {
				drag.fineRange = undefined;
			} else if (!drag.fineRange) {
				const sharedFineRange = windowAround(
					advanced.state.lensAnchorMs,
					precisionLensWindowMs(drag.origin.endMs - drag.origin.startMs, 10),
					durationMs,
				);
				drag.fineRange = {
					startMs: Math.max(sharedFineRange.startMs, drag.origin.startMs),
					endMs: Math.min(sharedFineRange.endMs, drag.origin.endMs),
				};
			}
			const constrained = constrainFineDragToWindow(
				advanced.state,
				drag.fineRange ?? drag.origin,
			);
			drag.fine = constrained;
			setDragFeedback({
				kind: { type: "playhead" },
				multiplier: constrained.multiplier,
				valueMs: constrained.valueMs,
				originMs: constrained.originMs,
				lensAnchorMs: constrained.lensAnchorMs,
				dyPx: event.clientY - drag.startYPx,
				startYPx: drag.startYPx,
				pointerXPx: event.clientX,
				pointerYPx: event.clientY,
				plotLeftPx: drag.leftPx,
				plotWidthPx: drag.widthPx,
				limitWindow: drag.fineRange,
			});
			if (props.onSeekPreview) props.onSeekPreview(constrained.valueMs);
			else props.onSeek(constrained.valueMs);
			if (
				constrained.multiplier > 1 &&
				rollingEdgeStrength(
					drag.pointerXPx,
					drag.leftPx,
					drag.widthPx,
					ROLLING_EDGE_ZONE_PX,
				) !== 0
			) {
				startEdgeDrive();
			}
			return;
		}
		if (!drag.moved && Math.abs(deltaPx) < VIEW_DRAG_THRESHOLD_PX) return;
		drag.moved = true;
		const nextView = shiftWindow(
			drag.origin,
			deltaPx * drag.msPerPx,
			durationMs,
		);
		synchronizeSelectionWithView(drag.originSelection, drag.origin, nextView);
		setView(nextView);
	};

	const finishViewDrag = (event: ReactPointerEvent<HTMLElement>) => {
		const drag = viewDragRef.current;
		if (!drag || drag.pointerId !== event.pointerId) return;
		event.stopPropagation();
		if (drag.kind === "detail") {
			continueViewDrag(event);
			const positionMs =
				drag.fine?.valueMs ??
				timeAtPointer(event.clientX, drag.leftPx, drag.widthPx, drag.origin);
			endViewDrag();
			props.onSeek(positionMs);
			return;
		}
		endViewDrag();
	};

	const cancelViewDrag = (event: ReactPointerEvent<HTMLElement>) => {
		if (viewDragRef.current?.pointerId === event.pointerId) {
			event.stopPropagation();
			endViewDrag();
		}
	};

	const viewDragHandlers = (kind: ViewDragKind) => ({
		onPointerDown: (event: ReactPointerEvent<HTMLElement>) =>
			beginViewDrag(event, kind),
		onPointerMove: continueViewDrag,
		onPointerUp: finishViewDrag,
		onPointerCancel: cancelViewDrag,
		onLostPointerCapture: cancelViewDrag,
	});

	return {
		plotRef,
		dragFeedback,
		precisionZone,
		precisionBoundaries,
		viewDragHandlers,
		viewDragging,
		view,
		peaks,
		startFraction,
		endFraction,
		stampFraction,
		selectionGeometry,
		valid,
		dragInvalid,
		selectionDrag,
		dragHandlers,
		onHandleKeyDown,
		suggestedEdge,
		displaySelection,
		playheadFraction,
		fineWindow,
		fineLimitWindow,
		rollingStrength,
		fineSelectionGeometry,
		otherEdgeFraction,
		fineValueFraction,
		fineLimitFraction,
		preciseAxis,
		selectionMs,
		canSetStart,
		canSetEnd,
		zoom,
	};
}

export type ClipRangeViewportController = ReturnType<
	typeof useClipRangeViewport
>;
