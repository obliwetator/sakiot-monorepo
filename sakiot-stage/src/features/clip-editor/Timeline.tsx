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
import { formatDuration } from "../../utils/formatTime";
import {
	axisLabelTransform,
	gridLineOffset,
	TIMELINE_AXIS_FRACTIONS,
	TimelinePlayhead,
	TimelineRow,
} from "../audio-dashboard/timelineLayout";
import type { ClipEdit, TimelineSegment } from "./model";
import { moveSegment, segmentEnd, setSegmentRange } from "./model";
import type { UseClipEditorReturn } from "./useClipEditor";

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

function parseDraggedClip(
	dataTransfer: DataTransfer,
): DraggedClipPayload | null {
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

export function Timeline(props: {
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

	useEffect(() => {
		const plot = plotRef.current;
		if (!plot) return;
		setPlotWidth(plot.clientWidth);
		const observer = new ResizeObserver((entries) => {
			setPlotWidth(entries[0]?.contentRect.width ?? 1);
		});
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

	const computeDrop = (clientX: number, clientY: number) => {
		const containerRect = tracksRef.current?.getBoundingClientRect() ?? {
			left: 0,
			width: 1,
		};
		const found = findTrackAtY(clientY);
		const track =
			found === null ? editor.edit.tracks : Math.min(found, editor.edit.tracks);
		const fractionOfWidth = Math.min(
			1,
			Math.max(
				0,
				(clientX - containerRect.left) / Math.max(1, containerRect.width),
			),
		);
		let startSec = editor.viewStartSec + fractionOfWidth * editor.viewWidthSec;
		if (Math.abs(startSec - editor.positionSec) < SNAP_SECONDS) {
			startSec = editor.positionSec;
		}
		return { track, startSec: Math.max(0, startSec) };
	};

	const handleDragOver = (event: ReactDragEvent<HTMLElement>) => {
		event.preventDefault();
		event.dataTransfer.dropEffect = "copy";
		const payload = parseDraggedClip(event.dataTransfer);
		if (!payload) return;
		const { track, startSec } = computeDrop(event.clientX, event.clientY);
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
		const { track, startSec } = computeDrop(event.clientX, event.clientY);
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
					onDeselect={() => editor.select(null)}
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
					{rows.map((track) => (
						<TrackRow
							key={track}
							track={track}
							editor={editor}
							clipName={props.clipName}
							fraction={fraction}
							pxPerSec={pxPerSec}
							preview={preview}
							onRowRef={(element) => {
								if (element) rowElementsRef.current.set(track, element);
								else rowElementsRef.current.delete(track);
							}}
						/>
					))}
					{preview && preview.track === editor.edit.tracks && (
						<PhantomTrackRow
							label={`Track ${editor.edit.tracks + 1}`}
							fraction={fraction}
							preview={preview}
						/>
					)}
				</Box>
			</Box>
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

function DragGhost(props: {
	leftFraction: number;
	widthFraction: number;
	label?: string;
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
				borderColor: "primary.light",
				bgcolor: "rgba(56, 189, 248, 0.16)",
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
	fraction: (sec: number) => number;
	preview: DragPreviewState;
}) {
	const start = props.fraction(props.preview.startSec);
	const width = Math.max(
		0,
		props.fraction(props.preview.startSec + props.preview.lengthSec) - start,
	);
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
				<DragGhost leftFraction={start} widthFraction={width} />
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

function TimelineRuler(props: {
	fraction: (sec: number) => number;
	positionSec: number;
	onScrub: (sec: number) => void;
	onDeselect: () => void;
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
					props.onDeselect();
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
	editor: UseClipEditorReturn;
	clipName: (segment: TimelineSegment) => string;
	fraction: (sec: number) => number;
	pxPerSec: number;
	preview: DragPreviewState | null;
	onRowRef: (element: HTMLElement | null) => void;
}) {
	const { editor, track } = props;
	const segments = editor.edit.segments.filter((s) => s.track === track);
	const showPreview = props.preview?.track === track;

	return (
		<TimelineRow label={`Track ${track + 1}`}>
			<Box
				ref={(element: HTMLDivElement | null) => props.onRowRef(element)}
				onPointerDown={(event) => {
					if (event.button !== 0) return;
					props.editor.select(null);
				}}
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
							name={props.clipName(segment)}
							selected={editor.selectedSegmentId === segment.id}
							leftFraction={start}
							widthFraction={width}
							pxPerSec={props.pxPerSec}
							positionSec={editor.positionSec}
							maxSource={
								editor.sourceDuration(segment.sourceId) ?? segment.sourceOut
							}
							maxTrack={editor.edit.tracks - 1}
							onSelect={() => editor.select(segment.id)}
							onGestureStart={editor.beginGesture}
							onGestureEnd={editor.endGesture}
							onPreview={editor.preview}
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
				<TimelinePlayhead percent={props.fraction(editor.positionSec)} />
			</Box>
		</TimelineRow>
	);
}

function TrackSegment(props: {
	segment: TimelineSegment;
	name: string;
	selected: boolean;
	leftFraction: number;
	widthFraction: number;
	pxPerSec: number;
	positionSec: number;
	maxSource: number;
	maxTrack: number;
	onSelect: () => void;
	onGestureStart: () => void;
	onGestureEnd: () => void;
	onPreview: (updater: (edit: ClipEdit) => ClipEdit) => void;
}) {
	const { segment } = props;
	const duration = segment.sourceOut - segment.sourceIn;
	const endSec = segment.timelineStart + duration;

	const snap = (value: number) =>
		Math.abs(value - props.positionSec) < SNAP_SECONDS
			? props.positionSec
			: value;

	const attachDrag = (
		element: HTMLElement,
		mode: "move" | "left" | "right",
		pointerId: number,
		startX: number,
		startY: number,
	) => {
		const originStart = segment.timelineStart;
		const originIn = segment.sourceIn;
		const originOut = segment.sourceOut;
		const originTrack = segment.track;

		const onMove = (event: PointerEvent) => {
			const dt = (event.clientX - startX) / props.pxPerSec;
			if (mode === "move") {
				const track = Math.max(
					0,
					Math.min(
						props.maxTrack,
						originTrack +
							Math.round((event.clientY - startY) / TRACK_HEIGHT_PX),
					),
				);
				props.onPreview((edit) =>
					moveSegment(edit, segment.id, snap(originStart + dt), track),
				);
				return;
			}
			if (mode === "left") {
				const nextIn = Math.max(0, Math.min(originOut - 0.05, originIn + dt));
				const nextStart = snap(originStart + (nextIn - originIn));
				props.onPreview((edit) => {
					const ranged = setSegmentRange(edit, segment.id, nextIn, originOut);
					return moveSegment(ranged, segment.id, nextStart, originTrack);
				});
				return;
			}
			const rawOut = Math.max(originIn + 0.05, originOut + dt);
			const rawEnd = endSec + (rawOut - originOut);
			const nextEnd = snap(rawEnd);
			const nextOut = originOut + (nextEnd - endSec);
			props.onPreview((edit) =>
				setSegmentRange(
					edit,
					segment.id,
					originIn,
					Math.min(props.maxSource, nextOut),
				),
			);
		};

		const onUp = () => {
			element.removeEventListener("pointermove", onMove);
			element.removeEventListener("pointerup", onUp);
			element.removeEventListener("pointercancel", onUp);
			props.onGestureEnd();
		};

		element.setPointerCapture(pointerId);
		element.addEventListener("pointermove", onMove);
		element.addEventListener("pointerup", onUp);
		element.addEventListener("pointercancel", onUp);
		props.onGestureStart();
	};

	const beginGesture = (
		event: ReactPointerEvent<HTMLElement>,
		mode: "move" | "left" | "right",
	) => {
		if (event.button !== 0) return;
		event.preventDefault();
		event.stopPropagation();
		props.onSelect();
		attachDrag(
			event.currentTarget,
			mode,
			event.pointerId,
			event.clientX,
			event.clientY,
		);
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
				}}
			>
				{props.name}
			</Typography>
		</Box>
	);
}
