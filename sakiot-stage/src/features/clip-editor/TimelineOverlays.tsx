import { cn } from "../../shared/ui";
import { TimelineRow } from "../audio-dashboard/timelineLayout";
import type { TimelineSegment } from "./model";
import type { SegmentDragState } from "./timelineDrag";
import type { UseClipEditorReturn } from "./useClipEditor";

const TRACK_HEIGHT_PX = 83;

export function DragGhost(props: {
	leftFraction: number;
	widthFraction: number;
	label?: string;
	invalid?: boolean;
}) {
	return (
		<div
			aria-hidden="true"
			data-testid="clip-drag-ghost"
			className={cn(
				"absolute top-2 bottom-2 rounded-[1px] border-2 border-dashed pointer-events-none z-5",
				props.invalid ? "border-danger" : "border-focus",
				props.invalid ? "bg-danger/12" : "bg-info/16",
			)}
			style={{
				left: `${props.leftFraction}%`,
				width: `max(2px, ${props.widthFraction}%)`,
			}}
		>
			{props.label && (
				<span className="text-xs leading-5 px-1 whitespace-nowrap overflow-hidden text-ellipsis block leading-[1.6]">
					{props.label}
				</span>
			)}
		</div>
	);
}

export function PhantomTrackRow(props: {
	label: string;
	ghosts: Array<{
		key: string;
		leftFraction: number;
		widthFraction: number;
		name?: string;
		invalid?: boolean;
	}>;
}) {
	return (
		<TimelineRow label={props.label}>
			<div
				className="relative mb-1 rounded-[1px] border-2 border-dashed border-primary-strong overflow-hidden"
				style={{ height: TRACK_HEIGHT_PX }}
			>
				{props.ghosts.map((ghost) => (
					<DragGhost
						key={ghost.key}
						leftFraction={ghost.leftFraction}
						widthFraction={ghost.widthFraction}
						label={ghost.name}
						invalid={ghost.invalid}
					/>
				))}
				<span className="text-muted text-xs leading-5 absolute top-0.5 left-1">
					New track
				</span>
			</div>
		</TimelineRow>
	);
}

export function clipNameOfDragged(
	drag: SegmentDragState,
	editor: UseClipEditorReturn,
	clipName: (segment: TimelineSegment) => string,
): string {
	const segment = editor.edit.segments.find((s) => s.id === drag.segmentId);
	return segment ? clipName(segment) : "";
}

export function FloatingDragChip(props: {
	name: string;
	x: number;
	y: number;
}) {
	return (
		<div
			aria-hidden="true"
			className="fixed -translate-x-1/2 translate-y-3.5 pointer-events-none z-1400 max-w-60 px-2 py-1 rounded-[1px] border border-dashed border-danger bg-danger/14 backdrop-blur-xs overflow-hidden"
			style={{ left: props.x, top: props.y }}
		>
			<span className="text-xs leading-5 truncate">
				{props.name || "Clip"} · release to cancel
			</span>
		</div>
	);
}

export function ClampedEdgeWarning(props: { x: number; y: number }) {
	return (
		<div
			aria-hidden="true"
			className="fixed -translate-x-1/2 translate-y-3.5 pointer-events-none z-1400 max-w-65 px-2 py-1 rounded-[1px] border border-warning bg-amber-500/14 backdrop-blur-xs"
			style={{ left: props.x, top: props.y }}
		>
			<span className="text-xs leading-5 truncate">
				Edge meets the next clip
			</span>
		</div>
	);
}

export function TrackCollisionWarning(props: { x: number; y: number }) {
	return (
		<div
			aria-hidden="true"
			className="fixed -translate-x-1/2 translate-y-3.5 pointer-events-none z-1400 max-w-80 px-2 py-1 rounded-[1px] border border-danger bg-danger/14 backdrop-blur-xs"
			style={{ left: props.x, top: props.y }}
		>
			<span className="text-xs leading-5 truncate">
				Cannot move: segments would overlap on the same track
			</span>
		</div>
	);
}
