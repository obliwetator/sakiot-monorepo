import LoopIcon from "@mui/icons-material/Loop";
import PlayArrowIcon from "@mui/icons-material/PlayArrow";
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
	applyEdge,
	beginFineDrag,
	changedEdge,
	DEFAULT_DETAIL_WINDOW_MS,
	type FineDragState,
	moveSelection,
	nudgeEdge,
	panWindowToInclude,
	type SelectionEdge,
	selectionFitsWindow,
	type TimeWindow,
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

type DragKind = { type: "edge"; edge: SelectionEdge } | { type: "band" };

interface DragSession {
	kind: DragKind;
	pointerId: number;
	startY: number;
	fine: FineDragState;
	origin: SessionSelection;
}

function percent(fraction: number): string {
	return `${Math.min(1, Math.max(0, fraction)) * 100}%`;
}

export function ClipRangeEditor(props: {
	sessionId: string;
	durationMs: number;
	selection: SessionSelection;
	onSelectionChange: (next: SessionSelection) => void;
	positionMs: number;
	onSeek: (positionMs: number) => void;
	onSetEdgeFromPlayhead: (edge: SelectionEdge) => void;
	onPreview: () => void;
	previewing: boolean;
	loop: boolean;
	onLoopChange: (loop: boolean) => void;
}) {
	const { durationMs, onSelectionChange, selection } = props;
	const plotRef = useRef<HTMLDivElement | null>(null);
	const dragRef = useRef<DragSession | null>(null);
	const [plotWidth, setPlotWidth] = useState(0);
	const [windowMs, setWindowMs] = useState(DEFAULT_DETAIL_WINDOW_MS);
	const [multiplier, setMultiplier] = useState<number | null>(null);
	const [view, setView] = useState<TimeWindow>(() =>
		// A fresh session is selected end to end, which says nothing about where
		// a clip will be, so start the view on the playhead instead.
		selectionFitsWindow(selection, DEFAULT_DETAIL_WINDOW_MS)
			? windowForSelection(selection, DEFAULT_DETAIL_WINDOW_MS, durationMs)
			: windowAround(props.positionMs, DEFAULT_DETAIL_WINDOW_MS, durationMs),
	);
	const previousSelectionRef = useRef(selection);
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
			if (moved === null) return current;
			return panWindowToInclude(
				current,
				moved === "start" ? selection[0] : selection[1],
				durationMs,
			);
		});
	}, [durationMs, selection, windowMs]);

	const zoom = useCallback((direction: 1 | -1) => {
		setWindowMs((current) => zoomDetailWindow(current, direction));
	}, []);

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

	const endDrag = useCallback(() => {
		dragRef.current = null;
		setMultiplier(null);
	}, []);

	useEffect(() => {
		const cancelOnEscape = (event: KeyboardEvent) => {
			const drag = dragRef.current;
			if (!drag || event.key !== "Escape") return;
			onSelectionChange(drag.origin);
			endDrag();
		};
		globalThis.addEventListener("keydown", cancelOnEscape);
		return () => globalThis.removeEventListener("keydown", cancelOnEscape);
	}, [endDrag, onSelectionChange]);

	const msPerPx = windowMsPerPixel(view, plotWidth);
	const { peaks } = useSessionWaveformPeaks(props.sessionId);
	const selectionMs = selection[1] - selection[0];
	const valid = isValidClipSelection(selection);
	const startFraction = windowFraction(selection[0], view);
	const endFraction = windowFraction(selection[1], view);
	const playheadFraction = windowFraction(props.positionMs, view);
	const preciseAxis = view.endMs - view.startMs <= PRECISE_AXIS_WINDOW_MS;

	const beginDrag = (event: ReactPointerEvent<HTMLElement>, kind: DragKind) => {
		if (event.button !== 0) return;
		event.preventDefault();
		event.stopPropagation();
		event.currentTarget.setPointerCapture(event.pointerId);
		const anchorMs =
			kind.type === "band"
				? selection[0]
				: selection[kind.edge === "start" ? 0 : 1];
		dragRef.current = {
			kind,
			pointerId: event.pointerId,
			startY: event.clientY,
			fine: beginFineDrag(event.clientX, anchorMs),
			origin: selection,
		};
		setMultiplier(1);
	};

	const continueDrag = (event: ReactPointerEvent<HTMLElement>) => {
		const drag = dragRef.current;
		if (!drag || drag.pointerId !== event.pointerId || msPerPx <= 0) return;
		const advanced = advanceFineDrag(drag.fine, {
			xPx: event.clientX,
			dyPx: event.clientY - drag.startY,
			msPerPx,
		});
		drag.fine = advanced.state;
		setMultiplier(advanced.state.multiplier);
		onSelectionChange(
			drag.kind.type === "edge"
				? applyEdge(selection, drag.kind.edge, advanced.valueMs, durationMs)
				: moveSelection(selection, advanced.valueMs - selection[0], durationMs),
		);
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
		onPointerUp: endDrag,
		onPointerCancel: endDrag,
		onLostPointerCapture: endDrag,
	});

	return (
		<Box component="section" aria-label="Clip range editor" sx={{ mb: 2 }}>
			<TimelineRow label="Session" sx={{ mb: 0.5 }}>
				<Box
					onClick={(event) => {
						const bounds = event.currentTarget.getBoundingClientRect();
						const fraction =
							(event.clientX - bounds.left) / Math.max(1, bounds.width);
						setView(
							windowAround(
								fraction * durationMs,
								view.endMs - view.startMs,
								durationMs,
							),
						);
					}}
					sx={{
						position: "relative",
						height: OVERVIEW_HEIGHT_PX,
						borderRadius: 0.5,
						bgcolor: "rgba(148, 163, 184, 0.11)",
						cursor: "pointer",
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
					sx={{ position: "relative", height: DETAIL_HEIGHT_PX }}
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
							onSeekFraction={(fraction) =>
								props.onSeek(
									view.startMs + fraction * (view.endMs - view.startMs),
								)
							}
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

					<Box
						{...dragHandlers({ type: "band" })}
						role="button"
						tabIndex={-1}
						aria-label="Move clip selection"
						sx={{
							position: "absolute",
							top: 0,
							bottom: 0,
							left: percent(startFraction),
							width: `max(2px, ${(endFraction - startFraction) * 100}%)`,
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

					{(["start", "end"] as const).map((edge) => {
						const fraction = edge === "start" ? startFraction : endFraction;
						const valueMs = edge === "start" ? selection[0] : selection[1];
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
							aria-hidden="true"
							sx={{
								position: "absolute",
								top: 0,
								bottom: 0,
								left: percent(playheadFraction),
								width: 2,
								transform: "translateX(-1px)",
								bgcolor: TIMELINE_PLAYHEAD_COLOR,
								boxShadow: TIMELINE_PLAYHEAD_SHADOW,
								pointerEvents: "none",
							}}
						/>
					)}

					{multiplier !== null && (
						<Chip
							size="small"
							label={`×${multiplier} · ${(msPerPx / multiplier / 1_000).toFixed(3)} s/px`}
							sx={{
								position: "absolute",
								top: 6,
								left: 6,
								pointerEvents: "none",
								fontVariantNumeric: "tabular-nums",
							}}
						/>
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
					{(["start", "end"] as const).map((edge) => (
						<Stack key={edge} direction="row" spacing={0.5} alignItems="center">
							<Tooltip
								title={`Set ${edge === "start" ? "in" : "out"} point to the playhead (${
									edge === "start" ? "I" : "O"
								})`}
							>
								<Button
									size="small"
									variant="outlined"
									onClick={() => props.onSetEdgeFromPlayhead(edge)}
								>
									Set {edge === "start" ? "in" : "out"}
								</Button>
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
					<Box sx={{ flex: 1 }} />
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
