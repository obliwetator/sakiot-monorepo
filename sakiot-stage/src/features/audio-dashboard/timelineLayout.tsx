import Box from "@mui/material/Box";
import type { SxProps, Theme } from "@mui/material/styles";
import Typography from "@mui/material/Typography";
import type { ReactNode } from "react";

// Every timeline row (waveform, scrubber, event lanes, axis) reserves the same
// label gutter, so one millisecond lands on the same x in all of them.
export const TIMELINE_GUTTER_WIDTH = { xs: 76, sm: 104 };
export const TIMELINE_AXIS_FRACTIONS = [0, 0.25, 0.5, 0.75, 1] as const;
export const TIMELINE_GRID_COLOR = "rgba(148, 163, 184, 0.16)";
export const TIMELINE_PLAYHEAD_COLOR = "#f8fafc";
export const TIMELINE_PLAYHEAD_SHADOW = "0 0 0 1px rgba(2, 6, 23, 0.75)";

/** Horizontal offset of an axis label so it stays inside the plot at the ends. */
export function axisLabelTransform(fraction: number): string {
	if (fraction <= 0) return "none";
	if (fraction >= 1) return "translateX(-100%)";
	return "translateX(-50%)";
}

/** A vertical line drawn at `fraction` without spilling out of the plot. */
export function gridLineOffset(fraction: number): string {
	if (fraction <= 0) return "0px";
	if (fraction >= 1) return "-1px";
	return "-0.5px";
}

export function TimelinePlayhead(props: { percent: number }) {
	return (
		<Box
			aria-hidden="true"
			// The position changes on every playback frame; inline style keeps
			// emotion from injecting a new <style> tag per frame.
			style={{ left: `${props.percent}%` }}
			sx={{
				position: "absolute",
				top: 0,
				bottom: 0,
				width: 2,
				transform: "translateX(-1px)",
				bgcolor: TIMELINE_PLAYHEAD_COLOR,
				boxShadow: TIMELINE_PLAYHEAD_SHADOW,
				pointerEvents: "none",
				zIndex: 6,
			}}
		/>
	);
}

/** Vertical guides at the same fractions the time axis is labelled with. */
export function TimelineGrid() {
	return (
		<Box
			aria-hidden="true"
			sx={{
				position: "absolute",
				inset: 0,
				pointerEvents: "none",
				zIndex: 0,
			}}
		>
			{TIMELINE_AXIS_FRACTIONS.map((fraction) => (
				<Box
					key={fraction}
					sx={{
						position: "absolute",
						top: 0,
						bottom: 0,
						left: `${fraction * 100}%`,
						ml: gridLineOffset(fraction),
						width: "1px",
						bgcolor: TIMELINE_GRID_COLOR,
					}}
				/>
			))}
		</Box>
	);
}

/** One gutter-aligned row: right-aligned label, then the shared plot column. */
export function TimelineRow(props: {
	label?: ReactNode;
	labelAlign?: "center" | "flex-start";
	children: ReactNode;
	sx?: SxProps<Theme>;
}) {
	const label =
		typeof props.label === "string" ? (
			<Typography
				variant="caption"
				color="text.secondary"
				noWrap
				title={props.label}
			>
				{props.label}
			</Typography>
		) : (
			props.label
		);

	return (
		<Box sx={{ display: "flex", minWidth: 0, ...props.sx }}>
			<Box
				sx={{
					width: TIMELINE_GUTTER_WIDTH,
					flex: "0 0 auto",
					// Wide enough that a slider thumb parked at 00:00 cannot touch the
					// label it sits next to.
					pr: 1.5,
					display: "flex",
					alignItems: props.labelAlign ?? "center",
					justifyContent: "flex-end",
					textAlign: "right",
					minWidth: 0,
				}}
			>
				{label}
			</Box>
			<Box sx={{ position: "relative", flex: 1, minWidth: 0 }}>
				{props.children}
			</Box>
		</Box>
	);
}
