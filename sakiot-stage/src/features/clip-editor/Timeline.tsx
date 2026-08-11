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
import { pendingBinDrag } from "./ClipBin";
import type { TimelineSegment } from "./model";
import { segmentEnd, snapToNeighbors } from "./model";
import {
	ClampedEdgeWarning,
	clipNameOfDragged,
	FloatingDragChip,
	PhantomTrackRow,
	TrackCollisionWarning,
} from "./TimelineOverlays";
import { TimelineRuler } from "./TimelineRuler";
import { type DragPreviewState, TrackRow } from "./TimelineTracks";
import {
	applySegmentDrag,
	clampPointToRect,
	dragGhostGeometries,
	marqueeIntersectsSegment,
	marqueeOverlayOffset,
	type SegmentDragState,
	transitionTimelineDrag,
} from "./timelineDrag";
import type { UseClipEditorReturn } from "./useClipEditor";

const TRACK_HEIGHT_PX = 72;
const SNAP_SECONDS = 0.25;

interface DraggedClipPayload {
	clipId: string;
	lengthSec: number;
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

/** A live marquee; the rectangle spans the tracks the drag crosses. */
interface MarqueeState {
	startX: number;
	startY: number;
	currentX: number;
	currentY: number;
	track: number;
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
		compute: (ghost, event) => {
			const container = tracksRef.current;
			const rect = container?.getBoundingClientRect() ?? null;
			const transition = transitionTimelineDrag(
				ghost,
				{
					clientX: event.clientX,
					clientY: event.clientY,
					containerRect: rect
						? { top: rect.top, bottom: rect.bottom, height: rect.height }
						: null,
				},
				pxPerSec,
				editor.positionSec,
				editor.edit.segments,
				trackAtClientY(event.clientY),
			);
			if (container && transition.scrollDeltaY !== 0) {
				container.scrollTop += transition.scrollDeltaY;
			}
			return transition.state;
		},
		onCommit: ({ ghost, origin }) => {
			// A rejected vertical move must not commit or touch the selection.
			if (ghost.trackCollision) return;
			if (ghost !== origin && ghost.valid) {
				editor.apply((edit) => applySegmentDrag(edit, ghost));
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
