import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
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
		<Box
			aria-hidden="true"
			data-testid="clip-drag-ghost"
			style={{
				left: `${props.leftFraction}%`,
				width: `max(2px, ${props.widthFraction}%)`,
			}}
			sx={{
				position: "absolute",
				top: 8,
				bottom: 8,
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
				{props.ghosts.map((ghost) => (
					<DragGhost
						key={ghost.key}
						leftFraction={ghost.leftFraction}
						widthFraction={ghost.widthFraction}
						label={ghost.name}
						invalid={ghost.invalid}
					/>
				))}
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

export function ClampedEdgeWarning(props: { x: number; y: number }) {
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

export function TrackCollisionWarning(props: { x: number; y: number }) {
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
				maxWidth: 320,
				px: 1,
				py: 0.5,
				borderRadius: 1,
				border: "1px solid",
				borderColor: "error.main",
				bgcolor: "rgba(248, 113, 113, 0.14)",
				backdropFilter: "blur(4px)",
			}}
		>
			<Typography variant="caption" noWrap>
				Cannot move: segments would overlap on the same track
			</Typography>
		</Box>
	);
}
