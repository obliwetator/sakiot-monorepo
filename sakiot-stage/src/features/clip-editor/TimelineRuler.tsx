import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import { useRef } from "react";
import { formatDuration } from "../../utils/formatTime";
import {
	axisLabelTransform,
	gridLineOffset,
	TIMELINE_AXIS_FRACTIONS,
	TimelinePlayhead,
	TimelineRow,
} from "../audio-dashboard/timelineLayout";

export function TimelineRuler(props: {
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
					event.currentTarget.setPointerCapture(event.pointerId);
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
				onPointerCancel={() => {
					scrubRef.current.active = false;
				}}
				onLostPointerCapture={() => {
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
					height: 32,
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
