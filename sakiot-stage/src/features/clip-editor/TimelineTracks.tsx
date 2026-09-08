import {
	X as CloseIcon,
	VolumeOff as VolumeOffIcon,
	Volume2 as VolumeUpIcon,
} from "lucide-react";
import { type PointerEvent as ReactPointerEvent, useState } from "react";
import { BaseDialog } from "../../shared/BaseDialog";
import { alpha, palette } from "../../shared/palette";
import {
	Button,
	cn,
	IconButton,
	Tooltip,
	TooltipTrigger,
} from "../../shared/ui";
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
			<div className="flex flex-col items-end gap-0.5 min-w-0 max-w-full">
				<span
					title={`Track ${props.track + 1}`}
					className="text-xs leading-5 truncate min-w-0 max-w-full"
				>
					Track {props.track + 1}
				</span>
				<div className="flex items-center gap-0.5">
					<TooltipTrigger delay={400}>
						<IconButton
							aria-label={props.muted ? "Unmute track" : "Mute track"}
							className="p-0.5 flex-none"
							size="sm"
							onPress={props.onToggleMute}
						>
							{props.muted ? (
								<VolumeOffIcon size={16} />
							) : (
								<VolumeUpIcon size={16} />
							)}
						</IconButton>
						<Tooltip>{props.muted ? "Unmute track" : "Mute track"}</Tooltip>
					</TooltipTrigger>
					<TooltipTrigger delay={400}>
						<IconButton
							aria-label="Remove track"
							className="p-0.5 flex-none"
							size="sm"
							isDisabled={!props.canRemove}
							onPress={requestRemove}
						>
							<CloseIcon size={16} />
						</IconButton>
						<Tooltip>
							{props.canRemove
								? props.clipCount > 0
									? "Remove track and confirm clip deletion"
									: "Remove track"
								: "At least one track is required"}
						</Tooltip>
					</TooltipTrigger>
				</div>
			</div>
			<BaseDialog
				open={confirmOpen}
				onClose={() => setConfirmOpen(false)}
				title={`Remove Track ${props.track + 1}?`}
				actions={
					<>
						<Button onPress={() => setConfirmOpen(false)}>Cancel</Button>
						<Button variant="danger" onPress={confirmRemove}>
							Remove track
						</Button>
					</>
				}
			>
				<p className="text-sm leading-6 text-slate-200">
					This track contains {props.clipCount} clip
					{props.clipCount === 1 ? "" : "s"}. Removing the track will also
					delete those clips from the edit. Do you want to continue?
				</p>
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
			<div
				ref={(element: HTMLDivElement | null) => props.onRowRef(element)}
				onClick={props.onActivate}
				onPointerDown={(event) => props.onBeginMarquee(event, track)}
				className={cn(
					"relative mb-1 rounded-[1px] outline-offset-1 cursor-pointer overflow-hidden touch-none",
					props.active ? "bg-info/9" : "bg-muted/6",
					props.active ? "outline outline-info/45" : "outline-none",
				)}
				style={{ height: TRACK_HEIGHT_PX }}
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
			</div>
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
			<div
				onPointerDown={(event) => beginGesture(event, "left")}
				className="absolute top-0 bottom-0 left-0 cursor-ew-resize"
				style={{ width: HANDLE_WIDTH_PX }}
			/>
			<div
				onPointerDown={(event) => beginGesture(event, "right")}
				className="absolute top-0 bottom-0 right-0 cursor-ew-resize"
				style={{ width: HANDLE_WIDTH_PX }}
			/>
		</>
	);
	const topInteractionBar = props.audacityStyleInteraction ? (
		<fieldset
			data-testid="segment-interaction-bar"
			aria-label={`Select or move ${props.name}`}
			onPointerDown={(event) => beginGesture(event, "move")}
			onDoubleClick={props.onSelect}
			className={cn(
				"absolute top-0 left-0 right-0 border-b border-white/30 cursor-grab z-6",
				props.selected ? "bg-fuchsia-500/22" : "bg-slate-900/30",
			)}
			style={{ height: SEGMENT_INTERACTION_BAR_HEIGHT_PX }}
		>
			{resizeHandles}
		</fieldset>
	) : null;

	return (
		<div
			onPointerDown={
				props.audacityStyleInteraction
					? undefined
					: (event) => beginGesture(event, "move")
			}
			onDoubleClick={
				props.audacityStyleInteraction ? undefined : props.onSelect
			}
			className={cn(
				"absolute top-2 bottom-2 rounded-[1px] select-none overflow-hidden",
				props.dragging ? "opacity-45" : "opacity-100",
				props.selected ? "bg-purple-500/65" : "bg-info/35",
				props.selected ? "border-2" : "border",
				props.selected ? "border-creative" : "border-info/55",
				props.selected
					? "shadow-[0_0_0_3px_rgba(217,70,239,0.35),0_2px_10px_rgba(2,6,23,0.6)]"
					: "shadow-[0_1px_3px_rgba(2,6,23,0.4)]",
				props.audacityStyleInteraction ? "cursor-crosshair" : "cursor-grab",
				props.selected ? "z-4" : "z-2",
			)}
			style={{
				left: `${props.leftFraction}%`,
				width: `max(2px, ${props.widthFraction}%)`,
				minWidth: HANDLE_WIDTH_PX * 2,
				paddingTop: props.audacityStyleInteraction
					? `${SEGMENT_INTERACTION_BAR_HEIGHT_PX}px`
					: 0,
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
				<svg
					aria-hidden="true"
					className="absolute inset-0 w-full h-full pointer-events-none z-5 animate-copied-dashes motion-reduce:animate-none"
				>
					<rect
						x="2"
						y="2"
						width="calc(100% - 4px)"
						height="calc(100% - 4px)"
						rx="4"
						fill="none"
						stroke={alpha(palette.sky300, 0.9)}
						strokeWidth="2"
						strokeDasharray="10 8"
					/>
				</svg>
			)}
			{props.audacityStyleInteraction ? topInteractionBar : resizeHandles}
			<span className="text-xs leading-5 px-1 whitespace-nowrap overflow-hidden text-ellipsis block leading-[1.6] relative z-1 text-shadow-[0_1px_3px_rgba(2,6,23,0.9)]">
				{props.name}
			</span>
		</div>
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
		<fieldset
			data-testid="merged-interaction-bar"
			aria-label="Select or move merged segment"
			onPointerDown={beginGesture}
			onDoubleClick={() =>
				editor.selectMany(members.map((member) => member.id))
			}
			className={cn(
				"absolute top-0 left-0 right-0 border-b border-white/30 cursor-grab z-6",
				props.selected ? "bg-fuchsia-500/22" : "bg-slate-900/30",
			)}
			style={{ height: SEGMENT_INTERACTION_BAR_HEIGHT_PX }}
		/>
	) : null;

	return (
		<fieldset
			onPointerDown={props.audacityStyleInteraction ? undefined : beginGesture}
			onDoubleClick={
				props.audacityStyleInteraction
					? undefined
					: () => editor.selectMany(members.map((member) => member.id))
			}
			aria-label={`Merged unit of ${members.length} clips`}
			className={cn(
				"absolute top-2 bottom-2 rounded-[1px] select-none overflow-hidden",
				props.dragging ? "opacity-45" : "opacity-100",
				props.selected ? "bg-purple-500/65" : "bg-teal-400/20",
				props.selected ? "border-2" : "border border-dashed",
				props.selected ? "border-creative" : "border-teal-400/55",
				props.selected
					? "shadow-[0_0_0_3px_rgba(217,70,239,0.35),0_2px_10px_rgba(2,6,23,0.6)]"
					: "shadow-[0_1px_3px_rgba(2,6,23,0.4)]",
				props.audacityStyleInteraction ? "cursor-crosshair" : "cursor-grab",
				props.selected ? "z-4" : "z-2",
			)}
			style={{
				left: `${props.leftFraction}%`,
				width: `max(2px, ${props.widthFraction}%)`,
				paddingTop: props.audacityStyleInteraction
					? `${SEGMENT_INTERACTION_BAR_HEIGHT_PX}px`
					: 0,
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
			<span className="text-xs leading-5 px-1 whitespace-nowrap overflow-hidden text-ellipsis block leading-[1.6] relative z-1 text-shadow-[0_1px_3px_rgba(2,6,23,0.9)]">
				{props.name}
				{members.length > 1 ? ` +${members.length - 1}` : ""}
			</span>
			<span className="text-muted text-xs leading-5 absolute top-0.5 right-1 text-[10px] leading-[1.4] text-shadow-[0_1px_3px_rgba(2,6,23,0.9)]">
				merged
			</span>
		</fieldset>
	);
}

function CopiedOutline() {
	return (
		<svg
			aria-hidden="true"
			className="absolute inset-0 w-full h-full pointer-events-none z-5 animate-copied-dashes motion-reduce:animate-none"
		>
			<rect
				x="2"
				y="2"
				width="calc(100% - 4px)"
				height="calc(100% - 4px)"
				rx="4"
				fill="none"
				stroke={alpha(palette.sky300, 0.9)}
				strokeWidth="2"
				strokeDasharray="10 8"
			/>
		</svg>
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
		<div
			aria-hidden="true"
			className="absolute top-0 bottom-0 pointer-events-none"
			style={{
				left: `${leftFraction}%`,
				width: `${Math.max(0, widthFraction)}%`,
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
		</div>
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
		? alpha(palette.slate300, 0.42)
		: props.selected
			? alpha(palette.white, 0.5)
			: alpha(palette.slate900, 0.5);
	return (
		<div
			aria-hidden="true"
			data-testid="effect-tail-overlay"
			className={cn(
				"absolute top-0 bottom-0 pointer-events-none z-3",
				props.muted ? "bg-slate-500/16" : "bg-slate-900/12",
			)}
			style={{
				left: `${fractions.startFraction * 100}%`,
				width: `${fractions.widthFraction * 100}%`,
				backgroundImage: `repeating-linear-gradient(135deg, ${stripeColor} 0 2px, transparent 2px 8px)`,
				borderLeft: `1px dashed ${stripeColor}`,
			}}
		/>
	);
}
