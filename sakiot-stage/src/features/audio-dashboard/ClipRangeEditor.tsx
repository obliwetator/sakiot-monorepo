import LoopIcon from "@mui/icons-material/Loop";
import PlayArrowIcon from "@mui/icons-material/PlayArrow";
import RestartAltIcon from "@mui/icons-material/RestartAlt";
import StopIcon from "@mui/icons-material/Stop";
import ZoomInIcon from "@mui/icons-material/ZoomIn";
import ZoomOutIcon from "@mui/icons-material/ZoomOut";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import IconButton from "@mui/material/IconButton";
import Stack from "@mui/material/Stack";
import Tooltip from "@mui/material/Tooltip";
import Typography from "@mui/material/Typography";
import {
	type KeyboardEvent as ReactKeyboardEvent,
	type PointerEvent as ReactPointerEvent,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import { formatDuration, formatDurationPrecise } from "../../utils/formatTime";
import {
	advanceFineDrag,
	applyEdgeWithinWindow,
	beginFineDrag,
	canSetSelectionEdge,
	changedEdge,
	constrainFineDragToLens,
	constrainFineDragToWindow,
	defaultDetailWindowMs,
	FINE_DRAG_START_PX,
	type FineDragState,
	moveSelectionWithinWindow,
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
	MAX_CLIP_DURATION_MS,
	MIN_CLIP_DURATION_MS,
	type SessionSelection,
} from "./logicalSessionSelection";
import {
	axisLabelTransform,
	gridLineOffset,
	TIMELINE_AXIS_FRACTIONS,
	TIMELINE_GRID_COLOR,
	TIMELINE_PLAYHEAD_COLOR,
	TIMELINE_PLAYHEAD_SHADOW,
	TimelineRow,
} from "./timelineLayout";
import { useSessionWaveformPeaks, WaveformCanvas } from "./WaveformCanvas";

const DETAIL_HEIGHT_PX = 88;
const OVERVIEW_HEIGHT_PX = 16;
const HANDLE_WIDTH_PX = 11;
const KEY_NUDGE_MS = 100;
const KEY_NUDGE_COARSE_MS = 1_000;
const PRECISE_AXIS_WINDOW_MS = 120_000;
const VIEW_DRAG_THRESHOLD_PX = 4;
const ROLLING_EDGE_ZONE_PX = 48;
const ROLLING_EDGE_MAX_PX_PER_SECOND = 240;
const FINE_AXIS_FRACTIONS = [0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1];

type DragKind = { type: "edge"; edge: SelectionEdge } | { type: "band" };
type DragFeedbackKind = DragKind | { type: "playhead" };

interface DragSession {
	kind: DragKind;
	pointerId: number;
	startX: number;
	startY: number;
	plotLeftPx: number;
	plotWidthPx: number;
	fine: FineDragState;
	origin: SessionSelection;
	/** Fixed boundary for a whole-band drag, captured when the pointer goes down. */
	movementWindow: TimeWindow;
	moved: boolean;
}

interface DragFeedback {
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

type ViewDragKind = "overview" | "detail";

interface ViewDragSession {
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

function percent(fraction: number): string {
	return `${Math.min(1, Math.max(0, fraction)) * 100}%`;
}

function signedSeconds(deltaMs: number): string {
	const seconds = deltaMs / 1_000;
	return `${seconds >= 0 ? "+" : "−"}${Math.abs(seconds).toFixed(1)}s`;
}

function timeAtPointer(
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

export function ClipRangeEditor(props: {
	sessionId: string;
	durationMs: number;
	selection: SessionSelection;
	/** One-time centre for an editor opened from a stamp. */
	initialFocusMs?: number;
	onSelectionChange: (next: SessionSelection) => void;
	positionMs: number;
	onSeek: (positionMs: number) => void;
	/** Shows a playhead position while scrubbing without reloading audio. */
	onSeekPreview?: (positionMs: number | null) => void;
	onSetEdgeFromPlayhead: (edge: SelectionEdge) => void;
	onSetNearestEdgeFromPlayhead: () => void;
	edgeHint?: string | null;
	onReset: () => void;
	onPreview: () => void;
	previewing: boolean;
	loop: boolean;
	onLoopChange: (loop: boolean) => void;
	/** Draw peaks from the compressed silence-free session timeline. */
	silenceFree?: boolean;
}) {
	const { durationMs, onSelectionChange, selection } = props;
	const defaultWindowMs = defaultDetailWindowMs(durationMs);
	const plotRef = useRef<HTMLDivElement | null>(null);
	const dragRef = useRef<DragSession | null>(null);
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

	const endDrag = useCallback(() => {
		dragRef.current = null;
		setDragFeedback(null);
	}, []);

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
			if (dragRef.current) endDrag();
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
	}, [endDrag, endViewDrag]);

	useEffect(() => {
		const cancelOnEscape = (event: KeyboardEvent) => {
			if (event.key !== "Escape") return;
			const drag = dragRef.current;
			if (drag) {
				draggedSelectionRef.current = drag.origin;
				onSelectionChange(drag.origin);
				endDrag();
			}
			if (viewDragRef.current) endViewDrag();
		};
		globalThis.addEventListener("keydown", cancelOnEscape);
		return () => globalThis.removeEventListener("keydown", cancelOnEscape);
	}, [endDrag, endViewDrag, onSelectionChange]);

	const msPerPx = windowMsPerPixel(view, plotWidth);
	const { peaks } = useSessionWaveformPeaks(
		props.sessionId,
		props.silenceFree ?? false,
	);
	const selectionMs = selection[1] - selection[0];
	const valid = isValidClipSelection(selection);
	const suggestedEdge = nearestSelectionEdge(selection, props.positionMs);
	const canSetStart = canSetSelectionEdge(selection, "start", props.positionMs);
	const canSetEnd = canSetSelectionEdge(selection, "end", props.positionMs);
	const startFraction = windowFraction(selection[0], view);
	const endFraction = windowFraction(selection[1], view);
	const selectionGeometry = selectionWindowGeometry(selection, view);
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
		? selectionWindowGeometry(selection, fineWindow)
		: null;
	const otherEdgeMs =
		dragFeedback?.kind.type === "edge"
			? selection[dragFeedback.kind.edge === "start" ? 1 : 0]
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
		event.currentTarget.setPointerCapture(event.pointerId);
		const plotBounds = plotRef.current?.getBoundingClientRect();
		const anchorMs =
			kind.type === "band"
				? selection[0]
				: selection[kind.edge === "start" ? 0 : 1];
		dragRef.current = {
			kind,
			pointerId: event.pointerId,
			startX: event.clientX,
			startY: event.clientY,
			plotLeftPx: plotBounds?.left ?? 0,
			plotWidthPx: plotBounds?.width ?? plotWidth,
			fine: beginFineDrag(event.clientX, anchorMs),
			origin: selection,
			movementWindow: view,
			moved: false,
		};
		setDragFeedback({
			kind,
			multiplier: 1,
			valueMs: anchorMs,
			originMs: anchorMs,
			lensAnchorMs: anchorMs,
			dyPx: 0,
			startYPx: event.clientY,
			pointerXPx: event.clientX,
			pointerYPx: event.clientY,
			plotLeftPx: plotBounds?.left ?? 0,
			plotWidthPx: plotBounds?.width ?? plotWidth,
		});
	};

	const continueDrag = (event: ReactPointerEvent<HTMLElement>) => {
		const drag = dragRef.current;
		if (!drag || drag.pointerId !== event.pointerId || msPerPx <= 0) return;
		event.stopPropagation();
		if (
			drag.kind.type === "band" &&
			!drag.moved &&
			Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) <
				VIEW_DRAG_THRESHOLD_PX
		) {
			return;
		}
		drag.moved = true;
		const advanced = advanceFineDrag(drag.fine, {
			xPx: event.clientX,
			dyPx: event.clientY - drag.startY,
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
						selection,
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
		const dragState =
			drag.kind.type === "band"
				? { ...constrained, valueMs: nextSelection[0] }
				: constrained;
		drag.fine = dragState;
		setDragFeedback({
			kind: drag.kind,
			multiplier: dragState.multiplier,
			valueMs: dragState.valueMs,
			originMs: dragState.originMs,
			lensAnchorMs: dragState.lensAnchorMs,
			dyPx: event.clientY - drag.startY,
			startYPx: drag.startY,
			pointerXPx: event.clientX,
			pointerYPx: event.clientY,
			plotLeftPx: drag.plotLeftPx,
			plotWidthPx: drag.plotWidthPx,
		});
		draggedSelectionRef.current = nextSelection;
		onSelectionChange(nextSelection);
	};

	const finishDrag = (event: ReactPointerEvent<HTMLElement>) => {
		const drag = dragRef.current;
		if (!drag || drag.pointerId !== event.pointerId) return;
		event.stopPropagation();
		const movementPx = Math.hypot(
			event.clientX - drag.startX,
			event.clientY - drag.startY,
		);
		if (
			drag.kind.type === "band" &&
			!drag.moved &&
			movementPx >= VIEW_DRAG_THRESHOLD_PX
		) {
			continueDrag(event);
		}
		if (drag.kind.type === "band" && !drag.moved) {
			const positionMs = timeAtPointer(
				event.clientX,
				drag.plotLeftPx,
				drag.plotWidthPx,
				view,
			);
			endDrag();
			props.onSeek(positionMs);
			onSelectionChange(
				setNearestSelectionEdge(drag.origin, positionMs, durationMs),
			);
			return;
		}
		endDrag();
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
		onPointerMove: continueDrag,
		onPointerUp: finishDrag,
		onPointerCancel: endDrag,
		onLostPointerCapture: endDrag,
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

	return (
		<Box component="section" aria-label="Clip range editor" sx={{ mb: 2 }}>
			{dragFeedback && precisionZone && dragFeedback.multiplier > 1 && (
				<>
					<Box
						aria-hidden="true"
						sx={{
							position: "fixed",
							top: precisionZone.topPx,
							left: dragFeedback.plotLeftPx,
							width: dragFeedback.plotWidthPx,
							height: Math.max(0, precisionZone.bottomPx - precisionZone.topPx),
							zIndex: 1_290,
							borderTop: "1px solid rgba(125, 211, 252, 0.5)",
							borderBottom: "1px solid rgba(125, 211, 252, 0.5)",
							bgcolor: "rgba(56, 189, 248, 0.035)",
							pointerEvents: "none",
						}}
					>
						<Chip
							size="small"
							label={
								dragFeedback.multiplier >= 100
									? "Ultra ×100"
									: dragFeedback.multiplier >= 10
										? "Fine ×10"
										: "Normal ×1"
							}
							sx={{
								position: "absolute",
								top: Math.min(
									Math.max(
										dragFeedback.pointerYPx - precisionZone.topPx - 13,
										4,
									),
									Math.max(
										4,
										precisionZone.bottomPx - precisionZone.topPx - 30,
									),
								),
								right: 8,
								fontVariantNumeric: "tabular-nums",
							}}
						/>
					</Box>
					{precisionBoundaries.map(
						(boundary) =>
							boundary.yPx >= 0 &&
							boundary.yPx <= globalThis.innerHeight && (
								<Box
									key={boundary.label}
									aria-hidden="true"
									sx={{
										position: "fixed",
										top: boundary.yPx - 15,
										left: dragFeedback.plotLeftPx,
										width: dragFeedback.plotWidthPx,
										height: 30,
										zIndex: 1_291,
										backdropFilter: "blur(7px)",
										background:
											"linear-gradient(180deg, rgba(2, 6, 23, 0), rgba(56, 189, 248, 0.18), rgba(2, 6, 23, 0))",
										borderTop: "1px solid rgba(125, 211, 252, 0.18)",
										borderBottom: "1px solid rgba(125, 211, 252, 0.18)",
										pointerEvents: "none",
									}}
								>
									<Typography
										variant="caption"
										sx={{
											position: "absolute",
											right: 8,
											top: 6,
											color: "primary.light",
											textShadow: "0 1px 2px rgba(2, 6, 23, 0.9)",
										}}
									>
										{boundary.label}
									</Typography>
								</Box>
							),
					)}
				</>
			)}
			<TimelineRow label="Session" sx={{ mb: 0.5 }}>
				<Box
					{...viewDragHandlers("overview")}
					sx={{
						position: "relative",
						height: OVERVIEW_HEIGHT_PX,
						borderRadius: 0.5,
						bgcolor: "rgba(148, 163, 184, 0.11)",
						cursor: viewDragging === "overview" ? "grabbing" : "grab",
						touchAction: "none",
						userSelect: "none",
						overflow: "hidden",
					}}
				>
					<Box
						aria-hidden="true"
						sx={{
							position: "absolute",
							top: 0,
							bottom: 0,
							left: percent(view.startMs / Math.max(1, durationMs)),
							width: `max(3px, ${
								((view.endMs - view.startMs) / Math.max(1, durationMs)) * 100
							}%)`,
							bgcolor: "primary.main",
							opacity: 0.55,
							borderRadius: 0.5,
						}}
					/>
					<Box
						aria-hidden="true"
						sx={{
							position: "absolute",
							top: 0,
							bottom: 0,
							left: percent(props.positionMs / Math.max(1, durationMs)),
							width: 2,
							transform: "translateX(-1px)",
							bgcolor: TIMELINE_PLAYHEAD_COLOR,
							boxShadow: TIMELINE_PLAYHEAD_SHADOW,
						}}
					/>
				</Box>
			</TimelineRow>

			<TimelineRow label="Clip window" labelAlign="flex-start">
				<Box
					ref={plotRef}
					{...viewDragHandlers("detail")}
					sx={{
						position: "relative",
						height: DETAIL_HEIGHT_PX,
						cursor: "ew-resize",
						touchAction: "none",
						userSelect: "none",
					}}
				>
					<Box
						sx={{
							position: "absolute",
							inset: 0,
							borderRadius: 1,
							overflow: "hidden",
							bgcolor: "rgba(168, 85, 247, 0.18)",
						}}
					>
						<WaveformCanvas
							peaks={peaks}
							height={DETAIL_HEIGHT_PX}
							label="Clip window waveform"
							startFraction={view.startMs / Math.max(1, durationMs)}
							endFraction={view.endMs / Math.max(1, durationMs)}
						/>
						{[
							{ key: "before", left: "0%", right: percent(1 - startFraction) },
							{ key: "after", left: percent(endFraction), right: "0%" },
						].map((mask) => (
							<Box
								key={mask.key}
								aria-hidden="true"
								sx={{
									position: "absolute",
									top: 0,
									bottom: 0,
									left: mask.left,
									right: mask.right,
									bgcolor: "rgba(2, 6, 23, 0.6)",
									pointerEvents: "none",
								}}
							/>
						))}
					</Box>

					{TIMELINE_AXIS_FRACTIONS.map((fraction) => (
						<Box
							key={fraction}
							aria-hidden="true"
							sx={{
								position: "absolute",
								top: 0,
								bottom: 0,
								left: percent(fraction),
								ml: gridLineOffset(fraction),
								width: "1px",
								bgcolor: TIMELINE_GRID_COLOR,
								pointerEvents: "none",
							}}
						/>
					))}

					{stampFraction !== null &&
						stampFraction >= 0 &&
						stampFraction <= 1 && (
							<Box
								aria-hidden="true"
								sx={{
									position: "absolute",
									top: 0,
									bottom: 0,
									left: percent(stampFraction),
									borderLeft: "1px dashed",
									borderColor: "warning.light",
									pointerEvents: "none",
									zIndex: 3,
								}}
							>
								<Typography
									variant="caption"
									sx={{
										position: "absolute",
										top: 2,
										left: 4,
										color: "warning.light",
										textShadow: "0 1px 2px rgba(2, 6, 23, 0.9)",
									}}
								>
									Stamp
								</Typography>
							</Box>
						)}

					{selectionGeometry.overlaps && (
						<Box
							{...dragHandlers({ type: "band" })}
							role="button"
							tabIndex={-1}
							aria-label="Move clip selection; click to set nearest edge"
							title="Drag to move the selection, or click to set the nearest edge"
							sx={{
								position: "absolute",
								top: 0,
								bottom: 0,
								left: percent(selectionGeometry.startFraction),
								width: `max(2px, ${
									(selectionGeometry.endFraction -
										selectionGeometry.startFraction) *
									100
								}%)`,
								bgcolor: valid
									? "rgba(56, 189, 248, 0.22)"
									: "rgba(248, 113, 113, 0.22)",
								borderTop: "2px solid",
								borderBottom: "2px solid",
								borderColor: valid ? "info.light" : "error.light",
								cursor: "grab",
								touchAction: "none",
								"&:active": { cursor: "grabbing" },
							}}
						/>
					)}

					{(["start", "end"] as const).map((edge) => {
						const fraction = edge === "start" ? startFraction : endFraction;
						const valueMs = edge === "start" ? selection[0] : selection[1];
						const handleVisible =
							edge === "start"
								? selectionGeometry.startHandleVisible
								: selectionGeometry.endHandleVisible;
						if (!handleVisible) return null;
						return (
							<Box
								key={edge}
								{...dragHandlers({ type: "edge", edge })}
								onKeyDown={(event) => onHandleKeyDown(event, edge)}
								role="slider"
								tabIndex={0}
								aria-label={
									edge === "start" ? "Clip in point" : "Clip out point"
								}
								aria-valuemin={0}
								aria-valuemax={durationMs}
								aria-valuenow={Math.round(valueMs)}
								aria-valuetext={formatDurationPrecise(valueMs / 1_000)}
								sx={{
									position: "absolute",
									top: -4,
									bottom: -4,
									left: percent(fraction),
									width: HANDLE_WIDTH_PX,
									ml: `${-HANDLE_WIDTH_PX / 2}px`,
									borderRadius: 1,
									bgcolor: valid ? "info.light" : "error.light",
									boxShadow: "0 1px 4px rgba(2,6,23,0.7)",
									cursor: "ew-resize",
									zIndex: 11,
									outline:
										edge === suggestedEdge
											? "2px solid rgba(125, 211, 252, 0.72)"
											: "none",
									outlineOffset: 2,
									touchAction: "none",
									display: "grid",
									placeItems: "center",
									"&::after": {
										content: '""',
										width: "3px",
										height: "40%",
										borderRadius: "2px",
										bgcolor: "rgba(2, 6, 23, 0.55)",
									},
									"&:focus-visible": {
										outline: "2px solid",
										outlineColor: "primary.light",
										outlineOffset: 2,
									},
								}}
							/>
						);
					})}

					{playheadFraction >= 0 && playheadFraction <= 1 && (
						<Box
							{...viewDragHandlers("detail")}
							role="slider"
							tabIndex={-1}
							aria-label="Clip playhead"
							aria-valuemin={Math.round(view.startMs)}
							aria-valuemax={Math.round(view.endMs)}
							aria-valuenow={Math.round(props.positionMs)}
							aria-valuetext={formatDurationPrecise(props.positionMs / 1_000)}
							sx={{
								position: "absolute",
								top: 0,
								bottom: 0,
								left: percent(playheadFraction),
								width: HANDLE_WIDTH_PX,
								ml: `${-HANDLE_WIDTH_PX / 2}px`,
								zIndex: 10,
								cursor: "ew-resize",
								touchAction: "none",
								"&::after": {
									content: '""',
									position: "absolute",
									top: 0,
									bottom: 0,
									left: "50%",
									width: 2,
									transform: "translateX(-1px)",
									bgcolor: TIMELINE_PLAYHEAD_COLOR,
									boxShadow: TIMELINE_PLAYHEAD_SHADOW,
								},
							}}
						/>
					)}

					{fineWindow && fineLimitWindow && dragFeedback && (
						<Box
							aria-hidden="true"
							sx={{
								position: "absolute",
								top: 4,
								height: DETAIL_HEIGHT_PX / 2,
								left: 4,
								right: 4,
								zIndex: 12,
								overflow: "hidden",
								border: "1px solid",
								borderColor: "primary.light",
								borderRadius: 1,
								bgcolor: "rgba(2, 6, 23, 0.94)",
								boxShadow: "0 4px 14px rgba(2, 6, 23, 0.55)",
								pointerEvents: "none",
								"&::before, &::after": {
									content: '""',
									position: "absolute",
									top: 0,
									bottom: 0,
									width: 56,
									zIndex: 5,
								},
								"&::before": {
									left: 0,
									background:
										"linear-gradient(90deg, rgba(2, 6, 23, 0.96), rgba(2, 6, 23, 0))",
								},
								"&::after": {
									right: 0,
									background:
										"linear-gradient(270deg, rgba(2, 6, 23, 0.96), rgba(2, 6, 23, 0))",
								},
							}}
						>
							{rollingStrength !== 0 && (
								<Box
									sx={{
										position: "absolute",
										top: 0,
										bottom: 0,
										left: rollingStrength < 0 ? 0 : "auto",
										right: rollingStrength > 0 ? 0 : "auto",
										width: ROLLING_EDGE_ZONE_PX,
										zIndex: 6,
										display: "grid",
										placeItems: "center",
										color: "primary.light",
										opacity: 0.45 + Math.abs(rollingStrength) * 0.55,
										background:
											rollingStrength < 0
												? "linear-gradient(90deg, rgba(56, 189, 248, 0.42), rgba(56, 189, 248, 0))"
												: "linear-gradient(270deg, rgba(56, 189, 248, 0.42), rgba(56, 189, 248, 0))",
									}}
								>
									{rollingStrength < 0 ? "←" : "→"}
								</Box>
							)}
							{fineSelectionGeometry?.overlaps && (
								<Box
									sx={{
										position: "absolute",
										top: 0,
										bottom: 0,
										left: percent(fineSelectionGeometry.startFraction),
										width: `${
											(fineSelectionGeometry.endFraction -
												fineSelectionGeometry.startFraction) *
											100
										}%`,
										bgcolor: valid
											? "rgba(56, 189, 248, 0.2)"
											: "rgba(248, 113, 113, 0.2)",
										borderTop: "2px solid",
										borderBottom: "2px solid",
										borderColor: valid ? "info.light" : "error.light",
										zIndex: 1,
									}}
								/>
							)}
							{FINE_AXIS_FRACTIONS.map((fraction) => (
								<Box
									key={fraction}
									sx={{
										position: "absolute",
										top:
											fraction === 0 || fraction === 0.5 || fraction === 1
												? 22
												: 30,
										bottom: 0,
										left: percent(fraction),
										ml: gridLineOffset(fraction),
										width: "1px",
										bgcolor: "rgba(226, 232, 240, 0.28)",
										zIndex: 2,
									}}
								/>
							))}
							{otherEdgeFraction !== null &&
								otherEdgeFraction >= 0 &&
								otherEdgeFraction <= 1 && (
									<Box
										sx={{
											position: "absolute",
											top: 18,
											bottom: 10,
											left: percent(otherEdgeFraction),
											borderLeft: "2px dashed",
											borderColor: "warning.light",
											zIndex: 4,
										}}
									>
										<Typography
											variant="caption"
											sx={{
												position: "absolute",
												top: -16,
												left: 3,
												color: "warning.light",
											}}
										>
											{dragFeedback.kind.type === "edge" &&
											dragFeedback.kind.edge === "start"
												? "Out"
												: "In"}
										</Typography>
									</Box>
								)}
							<Box
								sx={{
									position: "absolute",
									top: 18,
									bottom: 10,
									left: percent(fineValueFraction ?? 0),
									width: 3,
									transform: "translateX(-1px)",
									bgcolor: "info.light",
									boxShadow: TIMELINE_PLAYHEAD_SHADOW,
									zIndex: 7,
								}}
							/>
							<Chip
								size="small"
								label={`${
									fineLimitFraction !== null && fineLimitFraction <= 0
										? "← limit · "
										: fineLimitFraction !== null && fineLimitFraction >= 1
											? "limit → · "
											: ""
								}${
									dragFeedback.kind.type === "playhead"
										? "Head"
										: dragFeedback.kind.type === "band"
											? "Move"
											: dragFeedback.kind.edge === "start"
												? "In"
												: "Out"
								} · ${formatDurationPrecise(
									dragFeedback.valueMs / 1_000,
								)} · ${signedSeconds(
									dragFeedback.valueMs - dragFeedback.originMs,
								)}`}
								sx={{
									position: "absolute",
									top: 4,
									left: "50%",
									transform: "translateX(-50%)",
									fontVariantNumeric: "tabular-nums",
									zIndex: 8,
								}}
							/>
							<Typography
								variant="caption"
								sx={{
									position: "absolute",
									top: 5,
									left: 8,
									fontWeight: 700,
									color: "primary.light",
									zIndex: 8,
								}}
							>
								{dragFeedback.multiplier >= 100 ? "ULTRA ×100" : "FINE ×10"}
							</Typography>
							{dragFeedback.multiplier === 10 && (
								<Typography
									variant="caption"
									sx={{
										position: "absolute",
										top: 5,
										right: 8,
										color: "text.secondary",
										zIndex: 8,
									}}
								>
									↑{" "}
									{Math.max(
										0,
										ULTRA_FINE_DRAG_START_PX - Math.max(0, -dragFeedback.dyPx),
									).toFixed(0)}
									px to ultra
								</Typography>
							)}
							<Typography
								variant="caption"
								sx={{
									position: "absolute",
									left: 8,
									bottom: 3,
									fontVariantNumeric: "tabular-nums",
									zIndex: 8,
								}}
							>
								Start {formatDurationPrecise(fineLimitWindow.startMs / 1_000)}
							</Typography>
							<Typography
								variant="caption"
								sx={{
									position: "absolute",
									right: 8,
									bottom: 3,
									fontVariantNumeric: "tabular-nums",
									zIndex: 8,
								}}
							>
								End {formatDurationPrecise(fineLimitWindow.endMs / 1_000)}
							</Typography>
						</Box>
					)}
				</Box>
			</TimelineRow>

			<TimelineRow sx={{ mt: 0.5 }} labelAlign="flex-start">
				<Box sx={{ position: "relative", height: 18 }}>
					{TIMELINE_AXIS_FRACTIONS.map((fraction, index) => {
						const atMs = view.startMs + fraction * (view.endMs - view.startMs);
						return (
							<Box
								key={fraction}
								sx={{
									position: "absolute",
									top: 0,
									left: percent(fraction),
									display:
										index % 2 === 1 ? { xs: "none", sm: "block" } : "block",
								}}
							>
								<Typography
									variant="caption"
									color="text.secondary"
									sx={{
										display: "block",
										transform: axisLabelTransform(fraction),
										whiteSpace: "nowrap",
										fontVariantNumeric: "tabular-nums",
										lineHeight: 1.2,
									}}
								>
									{preciseAxis
										? formatDurationPrecise(atMs / 1_000)
										: formatDuration(atMs / 1_000)}
								</Typography>
							</Box>
						);
					})}
				</Box>
			</TimelineRow>

			<TimelineRow sx={{ mt: 1 }}>
				<Stack
					direction="row"
					spacing={1}
					alignItems="center"
					flexWrap="wrap"
					useFlexGap
				>
					<Typography
						variant="body2"
						sx={{ fontVariantNumeric: "tabular-nums" }}
					>
						In {formatDurationPrecise(selection[0] / 1_000)} · Out{" "}
						{formatDurationPrecise(selection[1] / 1_000)} · Length{" "}
						{(selectionMs / 1_000).toFixed(1)}s
					</Typography>
					<Chip
						size="small"
						color={valid ? "success" : "default"}
						variant={valid ? "filled" : "outlined"}
						label={
							valid
								? "Valid clip"
								: `Clip needs ${MIN_CLIP_DURATION_MS / 1_000}–${
										MAX_CLIP_DURATION_MS / 1_000
									}s`
						}
					/>
					<Typography variant="caption" color="text.secondary">
						Pull a handle or playhead upward while dragging for a magnified
						ruler. E sets the nearest edge · R resets the selection.
					</Typography>
					<Box sx={{ flex: 1 }} />
					<Tooltip title="Zoom out (ctrl + scroll)">
						<IconButton size="small" onClick={() => zoom(-1)}>
							<ZoomOutIcon fontSize="small" />
						</IconButton>
					</Tooltip>
					<Typography
						variant="caption"
						color="text.secondary"
						sx={{ fontVariantNumeric: "tabular-nums" }}
					>
						{formatDuration((view.endMs - view.startMs) / 1_000)}
					</Typography>
					<Tooltip title="Zoom in (ctrl + scroll)">
						<IconButton size="small" onClick={() => zoom(1)}>
							<ZoomInIcon fontSize="small" />
						</IconButton>
					</Tooltip>
				</Stack>
			</TimelineRow>

			<TimelineRow sx={{ mt: 1 }}>
				<Stack
					direction="row"
					spacing={1}
					alignItems="center"
					flexWrap="wrap"
					useFlexGap
				>
					<Tooltip
						title={`Set the ${suggestedEdge === "start" ? "left" : "right"} edge nearest the playhead (E)`}
					>
						<Button
							size="small"
							variant="contained"
							onClick={props.onSetNearestEdgeFromPlayhead}
						>
							Set nearest: {suggestedEdge === "start" ? "left" : "right"} (E)
						</Button>
					</Tooltip>
					{(["start", "end"] as const).map((edge) => (
						<Stack key={edge} direction="row" spacing={0.5} alignItems="center">
							<Tooltip
								title={
									(edge === "start" ? canSetStart : canSetEnd)
										? `Set the ${edge === "start" ? "left" : "right"} edge to the playhead (${edge === "start" ? "I" : "O"})`
										: `Move the playhead ${edge === "start" ? "left of the right" : "right of the left"} edge first`
								}
							>
								<span>
									<Button
										size="small"
										variant="outlined"
										disabled={edge === "start" ? !canSetStart : !canSetEnd}
										onClick={() => props.onSetEdgeFromPlayhead(edge)}
									>
										Set {edge === "start" ? "left edge (I)" : "right edge (O)"}
									</Button>
								</span>
							</Tooltip>
							{[-1_000, -100, 100, 1_000].map((deltaMs) => (
								<Button
									key={deltaMs}
									size="small"
									variant="text"
									sx={{ minWidth: 44, px: 0.5 }}
									onClick={() =>
										onSelectionChange(
											nudgeEdge(selection, edge, deltaMs, durationMs),
										)
									}
								>
									{deltaMs > 0 ? "+" : "−"}
									{Math.abs(deltaMs) / 1_000}s
								</Button>
							))}
						</Stack>
					))}
					{props.edgeHint && (
						<Typography
							variant="caption"
							color="warning.main"
							sx={{ flexBasis: "100%" }}
						>
							{props.edgeHint}
						</Typography>
					)}
					<Box sx={{ flex: 1 }} />
					<Tooltip title="Reset clip selection (R)">
						<Button
							size="small"
							variant="outlined"
							startIcon={<RestartAltIcon />}
							onClick={props.onReset}
						>
							Reset
						</Button>
					</Tooltip>
					<Button
						size="small"
						variant="contained"
						startIcon={props.previewing ? <StopIcon /> : <PlayArrowIcon />}
						onClick={props.onPreview}
					>
						{props.previewing ? "Stop" : "Preview"}
					</Button>
					<Button
						size="small"
						variant={props.loop ? "contained" : "outlined"}
						color={props.loop ? "secondary" : "primary"}
						startIcon={<LoopIcon />}
						aria-pressed={props.loop}
						onClick={() => props.onLoopChange(!props.loop)}
					>
						Loop
					</Button>
				</Stack>
			</TimelineRow>
		</Box>
	);
}
