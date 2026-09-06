import type { CSSProperties, ReactNode } from "react";
import { cn } from "../../shared/ui";

// Every timeline row (waveform, scrubber, event lanes, axis) reserves the same
// label gutter, so one millisecond lands on the same x in all of them.
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
		<div
			aria-hidden="true"
			className="absolute top-0 bottom-0 w-0.5 [transform:translateX(-1px)] pointer-events-none [z-index:6]"
			style={{
				backgroundColor: TIMELINE_PLAYHEAD_COLOR,
				boxShadow: TIMELINE_PLAYHEAD_SHADOW,
				...{ left: `${props.percent}%` },
			}}
		/>
	);
}

/** Vertical guides at the same fractions the time axis is labelled with. */
export function TimelineGrid() {
	return (
		<div
			aria-hidden="true"
			className="absolute inset-0 pointer-events-none [z-index:0]"
		>
			{TIMELINE_AXIS_FRACTIONS.map((fraction) => (
				<div
					key={fraction}
					className="absolute top-0 bottom-0 w-[1px]"
					style={{
						left: `${fraction * 100}%`,
						marginLeft: gridLineOffset(fraction),
						backgroundColor: TIMELINE_GRID_COLOR,
					}}
				/>
			))}
		</div>
	);
}

/** One gutter-aligned row: right-aligned label, then the shared plot column. */
export function TimelineRow(props: {
	label?: ReactNode;
	labelAlign?: "center" | "flex-start";
	children: ReactNode;
	style?: CSSProperties;
	className?: string;
}) {
	const label =
		typeof props.label === "string" ? (
			<span
				title={props.label}
				className="text-muted text-xs leading-5 truncate"
			>
				{props.label}
			</span>
		) : (
			props.label
		);

	return (
		<div className={cn("flex min-w-0", props.className)} style={props.style}>
			<div
				className="w-19 min-[600px]:w-26 flex-none pr-3 flex justify-end text-right min-w-0"
				style={{ alignItems: props.labelAlign ?? "center" }}
			>
				{label}
			</div>
			<div className="relative flex-1 min-w-0">{props.children}</div>
		</div>
	);
}
