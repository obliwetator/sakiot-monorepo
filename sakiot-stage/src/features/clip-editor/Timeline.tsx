import ZoomInIcon from "@mui/icons-material/ZoomIn";
import ZoomOutIcon from "@mui/icons-material/ZoomOut";
import Box from "@mui/material/Box";
import IconButton from "@mui/material/IconButton";
import Tooltip from "@mui/material/Tooltip";
import Typography from "@mui/material/Typography";
import type { PointerEvent as ReactPointerEvent } from "react";
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

export function Timeline(props: {
	editor: UseClipEditorReturn;
	clipName: (segment: TimelineSegment) => string;
	onDropClip: (
		track: number,
		clientX: number,
		element: HTMLElement,
		dataTransfer: DataTransfer,
	) => void;
}) {
	const { editor } = props;
	const plotRef = useRef<HTMLDivElement | null>(null);
	const [plotWidth, setPlotWidth] = useState(1);

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
							onDropClip={props.onDropClip}
						/>
					))}
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
	editor: UseClipEditorReturn;
	clipName: (segment: TimelineSegment) => string;
	fraction: (sec: number) => number;
	pxPerSec: number;
	onDropClip: (
		track: number,
		clientX: number,
		element: HTMLElement,
		dataTransfer: DataTransfer,
	) => void;
}) {
	const { editor, track } = props;
	const segments = editor.edit.segments.filter((s) => s.track === track);

	return (
		<TimelineRow label={`Track ${track + 1}`}>
			<Box
				onDragOver={(event) => {
					event.preventDefault();
					event.dataTransfer.dropEffect = "copy";
				}}
				onDrop={(event) => {
					event.preventDefault();
					props.onDropClip(
						track,
						event.clientX,
						event.currentTarget,
						event.dataTransfer,
					);
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
					? "rgba(168, 85, 247, 0.55)"
					: "rgba(56, 189, 248, 0.35)",
				border: "1px solid",
				borderColor: props.selected ? "secondary.main" : "primary.dark",
				cursor: "grab",
				userSelect: "none",
				overflow: "hidden",
				zIndex: 2,
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
