import CloseIcon from "@mui/icons-material/Close";
import VolumeOffIcon from "@mui/icons-material/VolumeOff";
import VolumeUpIcon from "@mui/icons-material/VolumeUp";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import DialogContentText from "@mui/material/DialogContentText";
import IconButton from "@mui/material/IconButton";
import { keyframes } from "@mui/material/styles";
import Tooltip from "@mui/material/Tooltip";
import Typography from "@mui/material/Typography";
import { type PointerEvent as ReactPointerEvent, useState } from "react";
import { BaseDialog } from "../../shared/BaseDialog";
import {
	TimelinePlayhead,
	TimelineRow,
} from "../audio-dashboard/timelineLayout";
import { EMPTY_WAVEFORM_ENVELOPE } from "../audio-dashboard/waveformPeaks";
import type { TimelineSegment } from "./model";
import {
	effectiveRate,
	effectTailFractions,
	segmentDuration,
	segmentEnd,
} from "./model";
import { SegmentWaveform } from "./SegmentWaveform";
import { DragGhost } from "./TimelineOverlays";
import type {
	GroupedSegment,
	SegmentDragMode,
	SegmentDragState,
} from "./timelineDrag";
import { dragGroupForSelection } from "./timelineDrag";
import type { UseClipEditorReturn } from "./useClipEditor";
import { useProcessedSegmentWaveform } from "./useProcessedSegmentWaveform";

const TRACK_HEIGHT_PX = 83;
const HANDLE_WIDTH_PX = 7;
const SEGMENT_INTERACTION_BAR_HEIGHT_PX = 12;
const copiedDashes = keyframes`
	from { stroke-dashoffset: 0; }
	to { stroke-dashoffset: -18; }
`;

export interface DragPreviewState {
	clipId: string;
	lengthSec: number;
	track: number;
	startSec: number;
}

function TrackLabel(props: {
	track: number;
	clipCount: number;
	muted: boolean;
	canRemove: boolean;
	onToggleMute: () => void;
	onRemove: () => void;
}) {
	const [confirmOpen, setConfirmOpen] = useState(false);
	const requestRemove = () => {
		if (props.clipCount > 0) {
			setConfirmOpen(true);
			return;
		}
		props.onRemove();
	};
	const confirmRemove = () => {
		setConfirmOpen(false);
		props.onRemove();
	};

	return (
		<>
			<Box
				sx={{
					display: "flex",
					flexDirection: "column",
					alignItems: "flex-end",
					gap: 0.25,
					minWidth: 0,
					maxWidth: "100%",
				}}
			>
				<Typography
					variant="caption"
					noWrap
					title={`Track ${props.track + 1}`}
					sx={{ minWidth: 0, maxWidth: "100%" }}
				>
					Track {props.track + 1}
				</Typography>
				<Box sx={{ display: "flex", alignItems: "center", gap: 0.25 }}>
					<Tooltip title={props.muted ? "Unmute track" : "Mute track"}>
						<IconButton
							size="small"
							aria-label={props.muted ? "Unmute track" : "Mute track"}
							color={props.muted ? "warning" : "default"}
							onClick={props.onToggleMute}
							sx={{ p: 0.25, flex: "0 0 auto" }}
						>
							{props.muted ? (
								<VolumeOffIcon fontSize="small" />
							) : (
								<VolumeUpIcon fontSize="small" />
							)}
						</IconButton>
					</Tooltip>
					<Tooltip
						title={
							props.canRemove
								? props.clipCount > 0
									? "Remove track and confirm clip deletion"
									: "Remove track"
								: "At least one track is required"
						}
					>
						<span>
							<IconButton
								size="small"
								aria-label="Remove track"
								disabled={!props.canRemove}
								onClick={requestRemove}
								sx={{ p: 0.25, flex: "0 0 auto" }}
							>
								<CloseIcon fontSize="small" />
							</IconButton>
						</span>
					</Tooltip>
				</Box>
			</Box>
			<BaseDialog
				open={confirmOpen}
				onClose={() => setConfirmOpen(false)}
				title={`Remove Track ${props.track + 1}?`}
				actions={
					<>
						<Button onClick={() => setConfirmOpen(false)}>Cancel</Button>
						<Button variant="contained" color="error" onClick={confirmRemove}>
							Remove track
						</Button>
					</>
				}
			>
				<DialogContentText>
					This track contains {props.clipCount} clip
					{props.clipCount === 1 ? "" : "s"}. Removing the track will also
					delete those clips from the edit. Do you want to continue?
				</DialogContentText>
			</BaseDialog>
		</>
	);
}

export function TrackRow(props: {
	track: number;
	guildId: string;
	editor: UseClipEditorReturn;
	clipName: (segment: TimelineSegment) => string;
	fraction: (sec: number) => number;
	pxPerSec: number;
	preview: DragPreviewState | null;
	active: boolean;
	muted: boolean;
	canRemove: boolean;
	audacityStyleInteraction: boolean;
	onActivate: () => void;
	onToggleMute: () => void;
	onRemoveTrack: () => void;
	dragGhosts: Array<{
		segmentId: string;
		leftFraction: number;
		widthFraction: number;
		name: string;
		invalid: boolean;
	}>;
	draggingSegmentIds: string[];
	onRowRef: (element: HTMLElement | null) => void;
	onBeginSegmentDrag: (drag: SegmentDragState) => void;
	onBeginMarquee: (
		event: ReactPointerEvent<HTMLElement>,
		track: number,
	) => void;
}) {
	const { editor, track } = props;
	const segments = editor.edit.segments.filter((s) => s.track === track);
	const showPreview = props.preview?.track === track;

	// Rows are made of individual segments plus one box per merged unit,
	// so a merged chain looks and behaves like a single clip.
	const membersByGroup = new Map<string, TimelineSegment[]>();
	for (const segment of segments) {
		if (!segment.mergeGroup) continue;
		const members = membersByGroup.get(segment.mergeGroup) ?? [];
		members.push(segment);
		membersByGroup.set(segment.mergeGroup, members);
	}
	type RowElement =
		| { kind: "segment"; segment: TimelineSegment }
		| { kind: "group"; members: TimelineSegment[] };
	const renderedGroups = new Set<string>();
	const elements: RowElement[] = [];
	for (const segment of segments) {
		if (!segment.mergeGroup) {
			elements.push({ kind: "segment", segment });
			continue;
		}
		if (renderedGroups.has(segment.mergeGroup)) continue;
		renderedGroups.add(segment.mergeGroup);
		elements.push({
			kind: "group",
			members: membersByGroup.get(segment.mergeGroup) ?? [segment],
		});
	}

	return (
		<TimelineRow
			label={
				<TrackLabel
					track={track}
					clipCount={segments.length}
					muted={props.muted}
					canRemove={props.canRemove}
					onToggleMute={props.onToggleMute}
					onRemove={props.onRemoveTrack}
				/>
			}
		>
			<Box
				ref={(element: HTMLDivElement | null) => props.onRowRef(element)}
				onClick={props.onActivate}
				onPointerDown={(event) => props.onBeginMarquee(event, track)}
				sx={{
					position: "relative",
					height: TRACK_HEIGHT_PX,
					mb: 0.5,
					borderRadius: 1,
					bgcolor: props.active
						? "rgba(56, 189, 248, 0.09)"
						: "rgba(148, 163, 184, 0.06)",
					outline: props.active ? "1px solid rgba(56, 189, 248, 0.45)" : "none",
					outlineOffset: 1,
					cursor: "pointer",
					overflow: "hidden",
					touchAction: "none",
				}}
			>
				{elements.map((element) => {
					if (element.kind === "group") {
						const first = element.members[0];
						if (!first) return null;
						const groupStartSec = Math.min(
							...element.members.map((member) => member.timelineStart),
						);
						const groupEndSec = Math.max(
							...element.members.map((member) => segmentEnd(member)),
						);
						const start = Math.min(
							...element.members.map((member) =>
								props.fraction(member.timelineStart),
							),
						);
						const end = Math.max(
							...element.members.map((member) =>
								props.fraction(segmentEnd(member)),
							),
						);
						const width = end - start;
						if (width <= 0) return null;
						return (
							<MergedUnitBox
								key={first.mergeGroup}
								members={element.members}
								first={first}
								guildId={props.guildId}
								editor={editor}
								name={props.clipName(first)}
								selected={element.members.some((member) =>
									editor.selectedSegmentIds.includes(member.id),
								)}
								copied={element.members.some((member) =>
									editor.copySourceIds.includes(member.id),
								)}
								dragging={element.members.some((member) =>
									props.draggingSegmentIds.includes(member.id),
								)}
								leftFraction={start}
								widthFraction={width}
								groupStartSec={groupStartSec}
								groupDurationSec={groupEndSec - groupStartSec}
								maxTrack={editor.edit.tracks - 1}
								muted={props.muted}
								audacityStyleInteraction={props.audacityStyleInteraction}
								onBeginDrag={props.onBeginSegmentDrag}
							/>
						);
					}
					const segment = element.segment;
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
							editor={editor}
							name={props.clipName(segment)}
							selected={editor.selectedSegmentIds.includes(segment.id)}
							copied={editor.copySourceIds.includes(segment.id)}
							dragging={props.draggingSegmentIds.includes(segment.id)}
							leftFraction={start}
							widthFraction={width}
							maxSource={
								editor.sourceDuration(segment.sourceId) ?? segment.sourceOut
							}
							maxTrack={editor.edit.tracks - 1}
							muted={props.muted}
							audacityStyleInteraction={props.audacityStyleInteraction}
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
				{props.dragGhosts.map((dragGhost) => (
					<DragGhost
						key={dragGhost.segmentId}
						leftFraction={dragGhost.leftFraction}
						widthFraction={dragGhost.widthFraction}
						label={dragGhost.name}
						invalid={dragGhost.invalid}
					/>
				))}
				<TimelinePlayhead percent={props.fraction(editor.positionSec)} />
			</Box>
		</TimelineRow>
	);
}

function TrackSegment(props: {
	segment: TimelineSegment;
	guildId: string;
	editor: UseClipEditorReturn;
	name: string;
	selected: boolean;
	copied: boolean;
	dragging: boolean;
	leftFraction: number;
	widthFraction: number;
	maxSource: number;
	maxTrack: number;
	muted: boolean;
	audacityStyleInteraction: boolean;
	onSelect: () => void;
	onBeginDrag: (drag: SegmentDragState) => void;
}) {
	const { segment, editor } = props;
	const waveform = useProcessedSegmentWaveform(
		props.guildId,
		segment,
		EMPTY_WAVEFORM_ENVELOPE,
	);
	const durationSec = props.maxSource > 0 ? props.maxSource : segment.sourceOut;

	const beginGesture = (
		event: ReactPointerEvent<HTMLElement>,
		mode: SegmentDragMode,
	) => {
		if (event.button !== 0) return;
		event.preventDefault();
		event.stopPropagation();
		// Capture so releasing outside the window still delivers the pointerup
		// (the drag hook listens on window, which misses it otherwise).
		try {
			event.currentTarget.setPointerCapture(event.pointerId);
		} catch {
			// Best-effort: the window listeners still cover the usual drag.
		}
		const selectedIds = editor.selectedSegmentIds;
		const selected = selectedIds.includes(segment.id);
		// Ctrl/Cmd-click toggles the segment in the selection: grabbing an
		// unselected segment adds it and drags the new selection; grabbing a
		// selected one removes it and drags only that segment.
		const modifierClick = event.ctrlKey || event.metaKey;
		let group: GroupedSegment[];
		if (modifierClick) {
			const next = editor.toggleSelect(segment.id);
			group = dragGroupForSelection(editor.edit.segments, next, segment.id);
		} else {
			// Grabbing a selected segment keeps the whole selection and moves
			// it as a group; grabbing anything else replaces the selection.
			// Edge trims only ever touch the grabbed segment.
			group = dragGroupForSelection(
				editor.edit.segments,
				selectedIds,
				segment.id,
			);
			if (!selected) props.onSelect();
		}
		props.onBeginDrag({
			mode,
			segmentId: segment.id,
			group,
			originStart: segment.timelineStart,
			originIn: segment.sourceIn,
			originOut: segment.sourceOut,
			originTrack: segment.track,
			originRate: effectiveRate(segment.effects),
			originTail: segment.effects.tailSeconds,
			reverse: segment.effects.reverse,
			maxSource: props.maxSource,
			maxTrack: props.maxTrack,
			startX: event.clientX,
			startY: event.clientY,
			ghostStart: segment.timelineStart,
			ghostIn: segment.sourceIn,
			ghostOut: segment.sourceOut,
			ghostTrack: segment.track,
			ghostStarts: [],
			modifierClick,
			trackCollision: false,
			valid: true,
			clamped: false,
			pointerX: event.clientX,
			pointerY: event.clientY,
		});
	};

	const resizeHandles = (
		<>
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
		</>
	);
	const topInteractionBar = props.audacityStyleInteraction ? (
		<Box
			data-testid="segment-interaction-bar"
			aria-label={`Select or move ${props.name}`}
			onPointerDown={(event) => beginGesture(event, "move")}
			onDoubleClick={props.onSelect}
			sx={{
				position: "absolute",
				top: 0,
				left: 0,
				right: 0,
				height: SEGMENT_INTERACTION_BAR_HEIGHT_PX,
				bgcolor: props.selected
					? "rgba(217, 70, 239, 0.22)"
					: "rgba(15, 23, 42, 0.3)",
				borderBottom: "1px solid rgba(255, 255, 255, 0.3)",
				cursor: "grab",
				zIndex: 6,
			}}
		>
			{resizeHandles}
		</Box>
	) : null;

	return (
		<Box
			onPointerDown={
				props.audacityStyleInteraction
					? undefined
					: (event) => beginGesture(event, "move")
			}
			onDoubleClick={
				props.audacityStyleInteraction ? undefined : props.onSelect
			}
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
				cursor: props.audacityStyleInteraction ? "crosshair" : "grab",
				userSelect: "none",
				overflow: "hidden",
				pt: props.audacityStyleInteraction
					? `${SEGMENT_INTERACTION_BAR_HEIGHT_PX}px`
					: 0,
				zIndex: props.selected ? 4 : 2,
			}}
		>
			<SegmentWaveform
				peaks={waveform.peaks}
				sourceIn={segment.sourceIn}
				sourceOut={segment.sourceOut}
				durationSec={durationSec}
				selected={props.selected}
				muted={props.muted}
				reverse={segment.effects.reverse}
				processed={waveform.processed}
			/>
			<EffectTailOverlay
				segment={segment}
				selected={props.selected}
				muted={props.muted}
			/>
			{props.copied && (
				<Box
					component="svg"
					aria-hidden="true"
					sx={{
						position: "absolute",
						inset: 0,
						width: "100%",
						height: "100%",
						pointerEvents: "none",
						zIndex: 5,
						animation: `${copiedDashes} 1s linear infinite`,
						"@media (prefers-reduced-motion: reduce)": {
							animation: "none",
						},
					}}
				>
					<rect
						x="2"
						y="2"
						width="calc(100% - 4px)"
						height="calc(100% - 4px)"
						rx="4"
						fill="none"
						stroke="rgba(125, 211, 252, 0.9)"
						strokeWidth="2"
						strokeDasharray="10 8"
					/>
				</Box>
			)}
			{props.audacityStyleInteraction ? topInteractionBar : resizeHandles}
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

/**
 * One box for a merged unit: spans its whole chain and moves as a rigid
 * group. The member segments keep their own sources and effects, so the
 * unit has no trim handles - ungroup (or undo) to edit the pieces.
 */
function MergedUnitBox(props: {
	members: TimelineSegment[];
	first: TimelineSegment;
	guildId: string;
	editor: UseClipEditorReturn;
	name: string;
	selected: boolean;
	copied: boolean;
	dragging: boolean;
	leftFraction: number;
	widthFraction: number;
	groupStartSec: number;
	groupDurationSec: number;
	maxTrack: number;
	muted: boolean;
	audacityStyleInteraction: boolean;
	onBeginDrag: (drag: SegmentDragState) => void;
}) {
	const { members, editor } = props;

	const beginGesture = (event: ReactPointerEvent<HTMLElement>) => {
		if (event.button !== 0) return;
		event.preventDefault();
		event.stopPropagation();
		try {
			event.currentTarget.setPointerCapture(event.pointerId);
		} catch {
			// Best-effort: the window listeners still cover the usual drag.
		}
		const toGrouped = (s: TimelineSegment): GroupedSegment => ({
			id: s.id,
			originStart: s.timelineStart,
			originTrack: s.track,
			duration: segmentDuration(s),
		});
		const first = props.first;
		// Selecting every member turns the existing multi-selection drag
		// machinery into a rigid group move.
		editor.selectMany(members.map((member) => member.id));
		props.onBeginDrag({
			mode: "move",
			segmentId: first.id,
			group: members.map(toGrouped),
			originStart: first.timelineStart,
			originIn: first.sourceIn,
			originOut: first.sourceOut,
			originTrack: first.track,
			originRate: effectiveRate(first.effects),
			originTail: first.effects.tailSeconds,
			reverse: first.effects.reverse,
			maxSource: first.sourceOut,
			maxTrack: props.maxTrack,
			startX: event.clientX,
			startY: event.clientY,
			ghostStart: first.timelineStart,
			ghostIn: first.sourceIn,
			ghostOut: first.sourceOut,
			ghostTrack: first.track,
			ghostStarts: [],
			modifierClick: event.ctrlKey || event.metaKey,
			trackCollision: false,
			valid: true,
			clamped: false,
			pointerX: event.clientX,
			pointerY: event.clientY,
		});
	};
	const topInteractionBar = props.audacityStyleInteraction ? (
		<Box
			data-testid="merged-interaction-bar"
			aria-label="Select or move merged segment"
			onPointerDown={beginGesture}
			onDoubleClick={() =>
				editor.selectMany(members.map((member) => member.id))
			}
			sx={{
				position: "absolute",
				top: 0,
				left: 0,
				right: 0,
				height: SEGMENT_INTERACTION_BAR_HEIGHT_PX,
				bgcolor: props.selected
					? "rgba(217, 70, 239, 0.22)"
					: "rgba(15, 23, 42, 0.3)",
				borderBottom: "1px solid rgba(255, 255, 255, 0.3)",
				cursor: "grab",
				zIndex: 6,
			}}
		/>
	) : null;

	return (
		<Box
			onPointerDown={props.audacityStyleInteraction ? undefined : beginGesture}
			onDoubleClick={
				props.audacityStyleInteraction
					? undefined
					: () => editor.selectMany(members.map((member) => member.id))
			}
			aria-label={`Merged unit of ${members.length} clips`}
			sx={{
				position: "absolute",
				top: 8,
				bottom: 8,
				left: `${props.leftFraction}%`,
				width: `max(2px, ${props.widthFraction}%)`,
				borderRadius: 1,
				opacity: props.dragging ? 0.45 : 1,
				bgcolor: props.selected
					? "rgba(168, 85, 247, 0.65)"
					: "rgba(45, 212, 191, 0.2)",
				border: props.selected ? "2px solid" : "1px dashed",
				borderColor: props.selected
					? "secondary.main"
					: "rgba(45, 212, 191, 0.55)",
				boxShadow: props.selected
					? "0 0 0 3px rgba(217, 70, 239, 0.35), 0 2px 10px rgba(2, 6, 23, 0.6)"
					: "0 1px 3px rgba(2, 6, 23, 0.4)",
				cursor: props.audacityStyleInteraction ? "crosshair" : "grab",
				userSelect: "none",
				overflow: "hidden",
				pt: props.audacityStyleInteraction
					? `${SEGMENT_INTERACTION_BAR_HEIGHT_PX}px`
					: 0,
				zIndex: props.selected ? 4 : 2,
			}}
		>
			{members.map((segment) => (
				<MergedMemberWaveform
					key={segment.id}
					guildId={props.guildId}
					editor={editor}
					segment={segment}
					groupStartSec={props.groupStartSec}
					groupDurationSec={props.groupDurationSec}
					selected={props.selected}
					muted={props.muted}
				/>
			))}
			{props.copied && <CopiedOutline />}
			{topInteractionBar}
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
				{members.length > 1 ? ` +${members.length - 1}` : ""}
			</Typography>
			<Typography
				variant="caption"
				color="text.secondary"
				sx={{
					position: "absolute",
					top: 2,
					right: 4,
					fontSize: 10,
					lineHeight: 1.4,
					textShadow: "0 1px 3px rgba(2, 6, 23, 0.9)",
				}}
			>
				merged
			</Typography>
		</Box>
	);
}

function CopiedOutline() {
	return (
		<Box
			component="svg"
			aria-hidden="true"
			sx={{
				position: "absolute",
				inset: 0,
				width: "100%",
				height: "100%",
				pointerEvents: "none",
				zIndex: 5,
				animation: `${copiedDashes} 1s linear infinite`,
				"@media (prefers-reduced-motion: reduce)": {
					animation: "none",
				},
			}}
		>
			<rect
				x="2"
				y="2"
				width="calc(100% - 4px)"
				height="calc(100% - 4px)"
				rx="4"
				fill="none"
				stroke="rgba(125, 211, 252, 0.9)"
				strokeWidth="2"
				strokeDasharray="10 8"
			/>
		</Box>
	);
}

/** Draws one locally processed member inside its merged unit's timeline span. */
function MergedMemberWaveform(props: {
	guildId: string;
	editor: UseClipEditorReturn;
	segment: TimelineSegment;
	groupStartSec: number;
	groupDurationSec: number;
	selected: boolean;
	muted: boolean;
}) {
	const { segment } = props;
	const waveform = useProcessedSegmentWaveform(
		props.guildId,
		segment,
		EMPTY_WAVEFORM_ENVELOPE,
	);
	const durationSec =
		props.editor.sourceDuration(segment.sourceId) ?? segment.sourceOut;
	const groupDuration = props.groupDurationSec;
	if (!Number.isFinite(groupDuration) || groupDuration <= 0) return null;

	const leftFraction =
		((segment.timelineStart - props.groupStartSec) / groupDuration) * 100;
	const widthFraction = (segmentDuration(segment) / groupDuration) * 100;
	if (!Number.isFinite(leftFraction) || !Number.isFinite(widthFraction)) {
		return null;
	}

	return (
		<Box
			aria-hidden="true"
			sx={{
				position: "absolute",
				top: 0,
				bottom: 0,
				left: `${leftFraction}%`,
				width: `${Math.max(0, widthFraction)}%`,
				pointerEvents: "none",
			}}
		>
			<SegmentWaveform
				peaks={waveform.peaks}
				sourceIn={segment.sourceIn}
				sourceOut={segment.sourceOut}
				durationSec={durationSec}
				selected={props.selected}
				muted={props.muted}
				reverse={segment.effects.reverse}
				processed={waveform.processed}
			/>
			<EffectTailOverlay
				segment={segment}
				selected={props.selected}
				muted={props.muted}
			/>
		</Box>
	);
}

/** Hatched overlay marking the silent duration added by an effect tail. */
function EffectTailOverlay(props: {
	segment: TimelineSegment;
	selected: boolean;
	muted: boolean;
}) {
	const fractions = effectTailFractions(props.segment);
	if (!fractions) return null;
	const stripeColor = props.muted
		? "rgba(203, 213, 225, 0.42)"
		: props.selected
			? "rgba(255, 255, 255, 0.5)"
			: "rgba(15, 23, 42, 0.5)";
	return (
		<Box
			aria-hidden="true"
			data-testid="effect-tail-overlay"
			sx={{
				position: "absolute",
				top: 0,
				bottom: 0,
				left: `${fractions.startFraction * 100}%`,
				width: `${fractions.widthFraction * 100}%`,
				backgroundColor: props.muted
					? "rgba(100, 116, 139, 0.16)"
					: "rgba(15, 23, 42, 0.12)",
				backgroundImage: `repeating-linear-gradient(135deg, ${stripeColor} 0 2px, transparent 2px 8px)`,
				borderLeft: `1px dashed ${stripeColor}`,
				pointerEvents: "none",
				zIndex: 3,
			}}
		/>
	);
}
