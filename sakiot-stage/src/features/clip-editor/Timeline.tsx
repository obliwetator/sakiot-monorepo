import ZoomInIcon from "@mui/icons-material/ZoomIn";
import ZoomOutIcon from "@mui/icons-material/ZoomOut";
import Box from "@mui/material/Box";
import IconButton from "@mui/material/IconButton";
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
	segmentEnd,
	setSegmentRange,
	snapToNeighbors,
} from "./model";
import { SegmentWaveform } from "./SegmentWaveform";
import type { UseClipEditorReturn } from "./useClipEditor";
import { useClipWaveform } from "./useClipWaveform";

const TRACK_HEIGHT_PX = 72;
const HANDLE_WIDTH_PX = 7;
const SNAP_SECONDS = 0.25;

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

interface SegmentDragState {
	mode: SegmentDragMode;
	segmentId: string;
	originStart: number;
	originIn: number;
	originOut: number;
	originTrack: number;
	originRate: number;
	maxSource: number;
	maxTrack: number;
	startX: number;
	startY: number;
	ghostStart: number;
	ghostIn: number;
	ghostOut: number;
	ghostTrack: number;
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

function computeGhost(
	drag: SegmentDragState,
	event: PointerEvent,
	pxPerSec: number,
	positionSec: number,
	container: HTMLElement | null,
	segments: readonly TimelineSegment[],
): SegmentDragState {
	const dt = (event.clientX - drag.startX) / Math.max(0.0001, pxPerSec);
	if (drag.mode === "move") {
		const rect = container?.getBoundingClientRect();
		const valid = rect
			? event.clientY >= rect.top && event.clientY <= rect.bottom
			: true;
		const rawStart = Math.max(0, drag.originStart + dt);
		const rawTrack =
			drag.originTrack +
			Math.round((event.clientY - drag.startY) / TRACK_HEIGHT_PX);
		const ghostTrack = Math.max(0, Math.min(drag.maxTrack + 1, rawTrack));
		const ghostStart = snapToNeighbors(
			snapTo(rawStart, positionSec),
			(drag.originOut - drag.originIn) / drag.originRate,
			segments,
			drag.segmentId,
			ghostTrack,
			rawStart,
		);
		return {
			...drag,
			ghostStart,
			ghostTrack,
			valid,
			clamped: false,
			pointerX: event.clientX,
			pointerY: event.clientY,
		};
	}
	if (drag.mode === "left") {
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
			drag.originStart + (drag.originOut - drag.originIn) / drag.originRate;
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
	const rawGhostOut = Math.max(
		drag.originIn + MIN_SEGMENT_SECONDS,
		Math.min(drag.maxSource, drag.originOut + dt * drag.originRate),
	);
	const endSec =
		drag.originStart + (drag.originOut - drag.originIn) / drag.originRate;
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
			: drag.originIn + (ceiling - drag.originStart) * drag.originRate;
	const clampedGhostOut = Math.min(ghostOut, maxGhostOut);
	return {
		...drag,
		ghostOut: clampedGhostOut,
		clamped: clampedGhostOut < ghostOut,
		pointerX: event.clientX,
		pointerY: event.clientY,
	};
}

function dragGhostGeometry(drag: SegmentDragState) {
	const duration = (drag.originOut - drag.originIn) / drag.originRate;
	if (drag.mode === "move") {
		return {
			startSec: drag.ghostStart,
			endSec: drag.ghostStart + duration,
		};
	}
	if (drag.mode === "left") {
		return {
			startSec: drag.ghostStart,
			endSec: drag.originStart + duration,
		};
	}
	return {
		startSec: drag.originStart,
		endSec: drag.originStart + (drag.ghostOut - drag.ghostIn) / drag.originRate,
	};
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
			),
		onCommit: ({ ghost, origin }) => {
			if (ghost !== origin && ghost.valid) {
				commitSegmentDrag(ghost, editor.apply);
			}
		},
	});
	const segmentDragGhost = segmentDrag.snapshot?.ghost ?? null;

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
					}}
				>
					{rows.map((track) => {
						const dragGhost =
							segmentDragGhost?.valid && segmentDragGhost.ghostTrack === track
								? dragGhostGeometry(segmentDragGhost)
								: null;
						const draggedSegment = segmentDragGhost
							? editor.edit.segments.find(
									(s) => s.id === segmentDragGhost.segmentId,
								)
							: null;
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
								dragGhost={
									dragGhost
										? {
												leftFraction: fraction(dragGhost.startSec),
												widthFraction: Math.max(
													0,
													fraction(dragGhost.endSec) -
														fraction(dragGhost.startSec),
												),
												name: draggedSegment
													? props.clipName(draggedSegment)
													: "",
												invalid: false,
											}
										: null
								}
								draggingSegmentId={segmentDragGhost?.segmentId ?? null}
								onRowRef={(element) => {
									if (element) rowElementsRef.current.set(track, element);
									else rowElementsRef.current.delete(track);
								}}
								onBeginSegmentDrag={(drag) =>
									segmentDrag.begin(drag, drag.startX, drag.startY)
								}
							/>
						);
					})}
					{(() => {
						const segmentDrop =
							segmentDragGhost?.valid &&
							segmentDragGhost.ghostTrack >= editor.edit.tracks
								? segmentDragGhost
								: null;
						const binDrop =
							preview && preview.track === editor.edit.tracks ? preview : null;
						if (!segmentDrop && !binDrop) return null;
						const startSec = segmentDrop?.ghostStart ?? binDrop?.startSec ?? 0;
						const durationSec = segmentDrop
							? segmentDrop.originOut - segmentDrop.originIn
							: (binDrop?.lengthSec ?? 0);
						const left = fraction(startSec);
						return (
							<PhantomTrackRow
								label={`Track ${editor.edit.tracks + 1}`}
								leftFraction={left}
								widthFraction={Math.max(
									0,
									fraction(startSec + durationSec) - left,
								)}
							/>
						);
					})()}
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
			const moved = moveSegment(
				edit,
				drag.segmentId,
				drag.ghostStart,
				drag.ghostTrack,
			);
			return drag.ghostTrack >= edit.tracks
				? { ...moved, tracks: drag.ghostTrack + 1 }
				: moved;
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
			sx={{
				position: "absolute",
				top: 8,
				bottom: 8,
				left: `${props.leftFraction}%`,
				width: `max(2px, ${props.widthFraction}%)`,
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
	leftFraction: number;
	widthFraction: number;
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
				<DragGhost
					leftFraction={props.leftFraction}
					widthFraction={props.widthFraction}
				/>
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
				onPointerLeave={() => {
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
					height: 22,
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
	dragGhost: {
		leftFraction: number;
		widthFraction: number;
		name: string;
		invalid: boolean;
	} | null;
	draggingSegmentId: string | null;
	onRowRef: (element: HTMLElement | null) => void;
	onBeginSegmentDrag: (drag: SegmentDragState) => void;
}) {
	const { editor, track } = props;
	const segments = editor.edit.segments.filter((s) => s.track === track);
	const showPreview = props.preview?.track === track;

	return (
		<TimelineRow label={`Track ${track + 1}`}>
			<Box
				ref={(element: HTMLDivElement | null) => props.onRowRef(element)}
				sx={{
					position: "relative",
					height: TRACK_HEIGHT_PX,
					mb: 0.5,
					borderRadius: 1,
					bgcolor: "rgba(148, 163, 184, 0.06)",
					overflow: "hidden",
					touchAction: "none",
				}}
			>
				{segments.map((segment) => {
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
							name={props.clipName(segment)}
							selected={editor.selectedSegmentId === segment.id}
							dragging={props.draggingSegmentId === segment.id}
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
				{props.dragGhost && (
					<DragGhost
						leftFraction={props.dragGhost.leftFraction}
						widthFraction={props.dragGhost.widthFraction}
						label={props.dragGhost.name}
						invalid={props.dragGhost.invalid}
					/>
				)}
				<TimelinePlayhead percent={props.fraction(editor.positionSec)} />
			</Box>
		</TimelineRow>
	);
}

function TrackSegment(props: {
	segment: TimelineSegment;
	guildId: string;
	name: string;
	selected: boolean;
	dragging: boolean;
	leftFraction: number;
	widthFraction: number;
	maxSource: number;
	maxTrack: number;
	onSelect: () => void;
	onBeginDrag: (drag: SegmentDragState) => void;
}) {
	const { segment } = props;
	const peaks = useClipWaveform(props.guildId, segment.sourceId);
	const durationSec = props.maxSource > 0 ? props.maxSource : segment.sourceOut;

	const beginGesture = (
		event: ReactPointerEvent<HTMLElement>,
		mode: SegmentDragMode,
	) => {
		if (event.button !== 0) return;
		event.preventDefault();
		event.stopPropagation();
		props.onSelect();
		props.onBeginDrag({
			mode,
			segmentId: segment.id,
			originStart: segment.timelineStart,
			originIn: segment.sourceIn,
			originOut: segment.sourceOut,
			originTrack: segment.track,
			originRate: effectiveRate(segment.effects),
			maxSource: props.maxSource,
			maxTrack: props.maxTrack,
			startX: event.clientX,
			startY: event.clientY,
			ghostStart: segment.timelineStart,
			ghostIn: segment.sourceIn,
			ghostOut: segment.sourceOut,
			ghostTrack: segment.track,
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
				peaks={peaks}
				sourceIn={segment.sourceIn}
				sourceOut={segment.sourceOut}
				durationSec={durationSec}
				selected={props.selected}
			/>
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
