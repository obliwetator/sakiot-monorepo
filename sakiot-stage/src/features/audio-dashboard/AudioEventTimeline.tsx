import { ChevronDown as ExpandMoreIcon } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import {
	Box,
	Button,
	Collapse,
	Popover,
	Tooltip,
	Typography,
} from "../../shared/ui";
import {
	type AudioTimelineEvent,
	buildEventTimelineModel,
	clusterTimelinePoints,
	formatTimelineOffset,
	type TimelineInterval,
	type TimelineLane,
	type TimelinePointCluster,
} from "./eventTimelineModel";
import {
	axisLabelTransform,
	gridLineOffset,
	TIMELINE_AXIS_FRACTIONS,
	TIMELINE_GRID_COLOR,
	TIMELINE_GUTTER_WIDTH,
	TimelineGrid,
	TimelinePlayhead,
	TimelineRow,
} from "./timelineLayout";

const TRACK_HEIGHT_PX = 18;
const LANE_PADDING_PX = 6;
const MIN_LANE_HEIGHT_PX = 30;
const CLUSTER_DISTANCE_PX = 14;
const AXIS_TICK_HEIGHT_PX = 4;
const LANE_RADIUS_PX = 4;

function laneHeight(lane: TimelineLane): number {
	return Math.max(
		MIN_LANE_HEIGHT_PX,
		lane.trackCount * TRACK_HEIGHT_PX + LANE_PADDING_PX * 2,
	);
}

function percent(positionMs: number, durationMs: number): number {
	return Math.min(100, Math.max(0, (positionMs / durationMs) * 100));
}

function formatWallClock(
	startedAtMs: number | undefined,
	offsetMs: number,
): string | null {
	if (startedAtMs === undefined || !Number.isFinite(startedAtMs)) return null;
	return new Date(startedAtMs + offsetMs).toLocaleString();
}

function eventContext(event: AudioTimelineEvent): string[] {
	const context: string[] = [];
	if (event.source) context.push(event.source.replaceAll("_", " "));
	if (event.previous_channel_id || event.channel_id) {
		context.push(
			`${event.previous_channel_id ?? "none"} → ${event.channel_id ?? "none"}`,
		);
	}
	if (event.details !== undefined && event.details !== null) {
		try {
			const details = JSON.stringify(event.details);
			if (details !== "{}" && details !== "[]" && details !== "null") {
				context.push(
					details.length > 180 ? `${details.slice(0, 177)}…` : details,
				);
			}
		} catch {
			// API details should be JSON. Ignore malformed diagnostic data.
		}
	}
	return context;
}

function PointTooltip(props: {
	cluster: TimelinePointCluster;
	startedAtMs?: number;
}) {
	if (props.cluster.points.length > 1) {
		return (
			<Box>
				<Typography variant="caption" fontWeight={700}>
					{props.cluster.points.length} nearby events
				</Typography>
				<Typography variant="caption" display="block">
					{formatTimelineOffset(props.cluster.startMs)} –{" "}
					{formatTimelineOffset(props.cluster.endMs)}
				</Typography>
				<Typography variant="caption" display="block">
					Click to choose exact event
				</Typography>
			</Box>
		);
	}

	const point = props.cluster.points[0];
	const wallClock = formatWallClock(props.startedAtMs, point.offsetMs);
	const context = eventContext(point.event);
	return (
		<Box>
			<Typography variant="caption" fontWeight={700}>
				{point.label}
			</Typography>
			<Typography variant="caption" display="block">
				{formatTimelineOffset(point.offsetMs)}
				{wallClock ? ` · ${wallClock}` : ""}
			</Typography>
			{context.map((line) => (
				<Typography key={line} variant="caption" display="block">
					{line}
				</Typography>
			))}
		</Box>
	);
}

function IntervalTooltip(props: {
	interval: TimelineInterval;
	startedAtMs?: number;
}) {
	const wallClock = formatWallClock(props.startedAtMs, props.interval.startMs);
	return (
		<Box>
			<Typography variant="caption" fontWeight={700}>
				{props.interval.label}
			</Typography>
			<Typography variant="caption" display="block">
				{formatTimelineOffset(props.interval.startMs)} –{" "}
				{formatTimelineOffset(props.interval.endMs)} ·{" "}
				{formatTimelineOffset(props.interval.endMs - props.interval.startMs)}
			</Typography>
			{wallClock && (
				<Typography variant="caption" display="block">
					Starts {wallClock}
				</Typography>
			)}
			{props.interval.startsAtBoundary && (
				<Typography variant="caption" display="block">
					State active when timeline begins
				</Typography>
			)}
			{props.interval.endsAtBoundary && (
				<Typography variant="caption" display="block">
					State continues to timeline end
				</Typography>
			)}
		</Box>
	);
}

function markerShape(laneId: TimelineLane["id"]): string {
	if (laneId === "connection") {
		return "polygon(50% 0, 100% 50%, 50% 100%, 0 50%)";
	}
	if (laneId === "channel") {
		return "polygon(0 0, 100% 0, 100% 72%, 50% 100%, 0 72%)";
	}
	if (laneId === "recording") {
		return "polygon(50% 0, 100% 100%, 0 100%)";
	}
	return "circle(50%)";
}

function clusterColor(cluster: TimelinePointCluster): string {
	const first = cluster.points[0].color;
	return cluster.points.every((point) => point.color === first)
		? first
		: "#64748b";
}

function ClusterPicker(props: {
	anchor: HTMLElement | null;
	cluster: TimelinePointCluster | null;
	startedAtMs?: number;
	onClose: () => void;
	onSeek: (offsetMs: number) => void;
}) {
	return (
		<Popover
			open={Boolean(props.anchor && props.cluster)}
			anchorEl={props.anchor}
			onClose={props.onClose}
			anchorOrigin={{ vertical: "bottom", horizontal: "center" }}
			transformOrigin={{ vertical: "top", horizontal: "center" }}
		>
			<Box sx={{ p: 1, maxHeight: 320, maxWidth: 360, overflowY: "auto" }}>
				<Typography variant="subtitle2" sx={{ px: 1, pb: 0.5 }}>
					{props.cluster?.points.length ?? 0} nearby events
				</Typography>
				{props.cluster?.points.map((point) => {
					const wallClock = formatWallClock(props.startedAtMs, point.offsetMs);
					return (
						<Button
							key={point.id}
							fullWidth
							size="small"
							onClick={() => {
								props.onSeek(point.offsetMs);
								props.onClose();
							}}
							sx={{
								justifyContent: "flex-start",
								textAlign: "left",
								textTransform: "none",
								gap: 1,
							}}
						>
							<Box
								aria-hidden="true"
								sx={{
									width: 9,
									height: 9,
									flex: "0 0 auto",
									bgcolor: point.color,
									clipPath: markerShape(point.laneId),
								}}
							/>
							<Box>
								<Typography variant="body2">{point.label}</Typography>
								<Typography variant="caption" color="text.secondary">
									{formatTimelineOffset(point.offsetMs)}
									{wallClock ? ` · ${wallClock}` : ""}
								</Typography>
							</Box>
						</Button>
					);
				})}
			</Box>
		</Popover>
	);
}

export function AudioEventTimeline(props: {
	events: readonly AudioTimelineEvent[];
	durationMs: number;
	positionMs?: number;
	startedAtMs?: number;
	onSeek: (offsetMs: number) => void;
}) {
	const plotRef = useRef<HTMLDivElement | null>(null);
	const contentId = useId();
	const [plotWidth, setPlotWidth] = useState(0);
	const [expanded, setExpanded] = useState(false);
	const [picker, setPicker] = useState<{
		anchor: HTMLElement;
		cluster: TimelinePointCluster;
	} | null>(null);
	const model = useMemo(
		() => buildEventTimelineModel(props.events, props.durationMs),
		[props.durationMs, props.events],
	);
	const clusters = useMemo(
		() =>
			clusterTimelinePoints(
				model.lanes.flatMap((lane) => lane.points),
				props.durationMs,
				plotWidth,
				CLUSTER_DISTANCE_PX,
			),
		[model.lanes, plotWidth, props.durationMs],
	);
	const intervalCount = model.lanes.reduce(
		(count, lane) => count + lane.intervals.length,
		0,
	);

	useEffect(() => {
		if (!expanded || model.lanes.length === 0) return;
		const plot = plotRef.current;
		if (!plot) return;
		setPlotWidth(plot.clientWidth);
		const observer = new ResizeObserver((entries) => {
			setPlotWidth(entries[0]?.contentRect.width ?? 0);
		});
		observer.observe(plot);
		return () => observer.disconnect();
	}, [expanded, model.lanes.length]);

	useEffect(() => {
		if (
			picker &&
			!clusters.some((cluster) => cluster.id === picker.cluster.id)
		) {
			setPicker(null);
		}
	}, [clusters, picker]);

	if (model.lanes.length === 0) return null;

	const clustersByLane = new Map<TimelineLane["id"], TimelinePointCluster[]>();
	for (const cluster of clusters) {
		const current = clustersByLane.get(cluster.laneId);
		if (current) current.push(cluster);
		else clustersByLane.set(cluster.laneId, [cluster]);
	}
	const playheadPercent =
		props.positionMs !== undefined && Number.isFinite(props.positionMs)
			? percent(props.positionMs, props.durationMs)
			: null;

	return (
		<Box
			component="section"
			aria-label="Recording event timeline"
			sx={{ minWidth: 0 }}
		>
			<Button
				fullWidth
				size="small"
				aria-expanded={expanded}
				aria-controls={contentId}
				onClick={() => {
					setExpanded((current) => !current);
					setPicker(null);
				}}
				sx={{
					minHeight: 24,
					height: 24,
					px: 0.75,
					py: 0,
					border: "1px solid rgba(148, 163, 184, 0.14)",
					borderRadius: 0.75,
					bgcolor: "rgba(148, 163, 184, 0.04)",
					color: "text.secondary",
					textTransform: "none",
					justifyContent: "stretch",
				}}
				style={{
					backgroundColor: "rgba(148, 163, 184, 0.04)",
					color: "var(--color-muted)",
				}}
			>
				<Box
					sx={{
						display: "flex",
						alignItems: "center",
						justifyContent: "space-between",
						gap: 1,
						width: "100%",
					}}
				>
					<Typography variant="caption" component="span" fontWeight={700}>
						Event timeline
					</Typography>
					<Box component="span" sx={{ display: "flex", alignItems: "center" }}>
						<Typography variant="caption" component="span" color="inherit">
							{model.totalEvents} event{model.totalEvents === 1 ? "" : "s"}
							{intervalCount > 0
								? ` · ${intervalCount} period${intervalCount === 1 ? "" : "s"}`
								: ""}
						</Typography>
						<ExpandMoreIcon
							size={17}
							style={{
								marginLeft: 2,
								transform: expanded ? "rotate(180deg)" : "none",
								transition: "transform 150ms ease",
							}}
						/>
					</Box>
				</Box>
			</Button>

			<Collapse in={expanded} unmountOnExit id={contentId}>
				<Box
					data-testid="event-timeline-content"
					sx={{
						display: "flex",
						minWidth: 0,
						mt: 0.5,
					}}
				>
					<Box
						aria-hidden="true"
						sx={{ width: TIMELINE_GUTTER_WIDTH, flex: "0 0 auto" }}
					>
						{model.lanes.map((lane) => (
							<Box
								key={lane.id}
								sx={{
									height: laneHeight(lane),
									display: "flex",
									alignItems: "center",
									justifyContent: "flex-end",
									pr: 1.5,
									minWidth: 0,
								}}
							>
								<Typography
									variant="caption"
									color="text.secondary"
									noWrap
									title={lane.label}
								>
									{lane.label}
								</Typography>
							</Box>
						))}
					</Box>

					<Box
						data-testid="event-timeline-plot"
						ref={plotRef}
						sx={{
							position: "relative",
							flex: 1,
							minWidth: 0,
						}}
					>
						{model.lanes.map((lane, laneIndex) => {
							const height = laneHeight(lane);
							const isFirstLane = laneIndex === 0;
							const isLastLane = laneIndex === model.lanes.length - 1;
							return (
								<Box
									key={lane.id}
									sx={{
										position: "relative",
										height,
										bgcolor:
											laneIndex % 2 === 0
												? "rgba(148, 163, 184, 0.11)"
												: "rgba(148, 163, 184, 0.04)",
										boxShadow: isLastLane
											? "none"
											: "inset 0 -1px 0 rgba(148, 163, 184, 0.14)",
										borderTopLeftRadius: isFirstLane ? LANE_RADIUS_PX : 0,
										borderTopRightRadius: isFirstLane ? LANE_RADIUS_PX : 0,
										borderBottomLeftRadius: isLastLane ? LANE_RADIUS_PX : 0,
										borderBottomRightRadius: isLastLane ? LANE_RADIUS_PX : 0,
									}}
								>
									{lane.intervals.map((interval) => {
										const left = percent(interval.startMs, props.durationMs);
										const width = percent(
											interval.endMs - interval.startMs,
											props.durationMs,
										);
										const widthPx = (width / 100) * plotWidth;
										return (
											<Tooltip
												key={interval.id}
												title={
													<IntervalTooltip
														interval={interval}
														startedAtMs={props.startedAtMs}
													/>
												}
												arrow
											>
												<Box
													component="button"
													type="button"
													aria-label={`${interval.label}, ${formatTimelineOffset(interval.startMs)} to ${formatTimelineOffset(interval.endMs)}`}
													onClick={(event) => {
														const bounds =
															event.currentTarget.getBoundingClientRect();
														const fraction =
															(event.clientX - bounds.left) /
															Math.max(1, bounds.width);
														props.onSeek(
															interval.startMs +
																fraction * (interval.endMs - interval.startMs),
														);
													}}
													sx={{
														position: "absolute",
														left: `${left}%`,
														top:
															LANE_PADDING_PX +
															interval.track * TRACK_HEIGHT_PX,
														width: `max(3px, ${width}%)`,
														height: TRACK_HEIGHT_PX - 6,
														p: 0,
														px: widthPx >= 60 ? 0.75 : 0,
														overflow: "hidden",
														border: 0,
														// Rounded ends mean the state starts and stops
														// inside the recording; square ends mean it runs
														// past that edge of the timeline.
														borderTopLeftRadius: interval.startsAtBoundary
															? 0
															: 999,
														borderBottomLeftRadius: interval.startsAtBoundary
															? 0
															: 999,
														borderTopRightRadius: interval.endsAtBoundary
															? 0
															: 999,
														borderBottomRightRadius: interval.endsAtBoundary
															? 0
															: 999,
														bgcolor: interval.color,
														boxShadow:
															"inset 0 1px 0 rgba(255,255,255,0.35), 0 1px 2px rgba(2,6,23,0.5)",
														color: "#0f172a",
														cursor: "pointer",
														fontSize: 10,
														fontWeight: 700,
														lineHeight: 1,
														textAlign: "left",
														whiteSpace: "nowrap",
														textOverflow: "ellipsis",
														zIndex: 1,
														"&:hover, &:focus-visible": {
															filter: "brightness(1.14)",
															zIndex: 4,
														},
													}}
												>
													{widthPx >= 60 ? interval.label : ""}
												</Box>
											</Tooltip>
										);
									})}

									{(clustersByLane.get(lane.id) ?? []).map((cluster) => {
										const clustered = cluster.points.length > 1;
										const markerSize = clustered ? 16 : 11;
										return (
											<Tooltip
												key={cluster.id}
												title={
													<PointTooltip
														cluster={cluster}
														startedAtMs={props.startedAtMs}
													/>
												}
												arrow
											>
												<Box
													component="button"
													type="button"
													aria-label={
														clustered
															? `${cluster.points.length} events between ${formatTimelineOffset(cluster.startMs)} and ${formatTimelineOffset(cluster.endMs)}`
															: `${cluster.points[0].label} at ${formatTimelineOffset(cluster.offsetMs)}`
													}
													onClick={(event) => {
														if (clustered) {
															setPicker({
																anchor: event.currentTarget,
																cluster,
															});
														} else {
															props.onSeek(cluster.points[0].offsetMs);
														}
													}}
													sx={{
														position: "absolute",
														// No clamping: the marker must sit exactly above
														// the same instant on the waveform and scrubber.
														left: `${percent(cluster.offsetMs, props.durationMs)}%`,
														top:
															LANE_PADDING_PX +
															cluster.track * TRACK_HEIGHT_PX +
															TRACK_HEIGHT_PX / 2,
														transform: "translate(-50%, -50%)",
														width: markerSize,
														height: markerSize,
														p: 0,
														border: 0,
														borderRadius: clustered ? "50%" : 0,
														clipPath: clustered
															? undefined
															: markerShape(cluster.laneId),
														bgcolor: clusterColor(cluster),
														color: "white",
														cursor: "pointer",
														fontSize: 9,
														fontWeight: 800,
														lineHeight: `${markerSize}px`,
														boxShadow:
															"0 0 0 1.25px rgba(255,255,255,0.85), 0 1px 3px rgba(2,6,23,0.7)",
														zIndex: 3,
														"&:hover, &:focus-visible": {
															transform: "translate(-50%, -50%) scale(1.2)",
															zIndex: 5,
														},
													}}
												>
													{clustered ? cluster.points.length : ""}
												</Box>
											</Tooltip>
										);
									})}
								</Box>
							);
						})}

						{/* After the lanes so the guides sit above their fills, but below
					    the markers, which own the foreground. */}
						<TimelineGrid />

						{playheadPercent !== null && (
							<TimelinePlayhead percent={playheadPercent} />
						)}
					</Box>
				</Box>

				<TimelineRow sx={{ mt: 0.5 }} labelAlign="flex-start">
					<Box sx={{ position: "relative", height: 18 }}>
						{TIMELINE_AXIS_FRACTIONS.map((fraction, index) => (
							<Box
								key={fraction}
								sx={{
									position: "absolute",
									top: 0,
									left: `${fraction * 100}%`,
									// Quarter marks crowd the narrow layout; the ends and the
									// midpoint stay readable at every width.
									display:
										index % 2 === 1 ? { xs: "none", sm: "block" } : "block",
								}}
							>
								<Box
									aria-hidden="true"
									sx={{
										position: "absolute",
										top: 0,
										left: 0,
										ml: gridLineOffset(fraction),
										width: "1px",
										height: AXIS_TICK_HEIGHT_PX,
										bgcolor: TIMELINE_GRID_COLOR,
									}}
								/>
								<Typography
									variant="caption"
									color="text.secondary"
									sx={{
										display: "block",
										mt: `${AXIS_TICK_HEIGHT_PX}px`,
										transform: axisLabelTransform(fraction),
										whiteSpace: "nowrap",
										fontVariantNumeric: "tabular-nums",
										lineHeight: 1.2,
									}}
								>
									{formatTimelineOffset(fraction * props.durationMs)}
								</Typography>
							</Box>
						))}
					</Box>
				</TimelineRow>

				<ClusterPicker
					anchor={picker?.anchor ?? null}
					cluster={picker?.cluster ?? null}
					startedAtMs={props.startedAtMs}
					onClose={() => setPicker(null)}
					onSeek={props.onSeek}
				/>
			</Collapse>
		</Box>
	);
}
