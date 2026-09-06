import { ChevronDown as ExpandMoreIcon } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import {
	Button,
	cn,
	Focusable,
	Popover,
	Tooltip,
	TooltipTrigger,
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
			<div>
				<span className="[font-weight:700] text-xs leading-5">
					{props.cluster.points.length} nearby events
				</span>
				<span className="block text-xs leading-5">
					{formatTimelineOffset(props.cluster.startMs)} –{" "}
					{formatTimelineOffset(props.cluster.endMs)}
				</span>
				<span className="block text-xs leading-5">
					Click to choose exact event
				</span>
			</div>
		);
	}

	const point = props.cluster.points[0];
	const wallClock = formatWallClock(props.startedAtMs, point.offsetMs);
	const context = eventContext(point.event);
	return (
		<div>
			<span className="[font-weight:700] text-xs leading-5">{point.label}</span>
			<span className="block text-xs leading-5">
				{formatTimelineOffset(point.offsetMs)}
				{wallClock ? ` · ${wallClock}` : ""}
			</span>
			{context.map((line) => (
				<span key={line} className="block text-xs leading-5">
					{line}
				</span>
			))}
		</div>
	);
}

function IntervalTooltip(props: {
	interval: TimelineInterval;
	startedAtMs?: number;
}) {
	const wallClock = formatWallClock(props.startedAtMs, props.interval.startMs);
	return (
		<div>
			<span className="[font-weight:700] text-xs leading-5">
				{props.interval.label}
			</span>
			<span className="block text-xs leading-5">
				{formatTimelineOffset(props.interval.startMs)} –{" "}
				{formatTimelineOffset(props.interval.endMs)} ·{" "}
				{formatTimelineOffset(props.interval.endMs - props.interval.startMs)}
			</span>
			{wallClock && (
				<span className="block text-xs leading-5">Starts {wallClock}</span>
			)}
			{props.interval.startsAtBoundary && (
				<span className="block text-xs leading-5">
					State active when timeline begins
				</span>
			)}
			{props.interval.endsAtBoundary && (
				<span className="block text-xs leading-5">
					State continues to timeline end
				</span>
			)}
		</div>
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
			isOpen={Boolean(props.anchor && props.cluster)}
			onOpenChange={(isOpen) => {
				if (!isOpen) props.onClose();
			}}
			triggerRef={{ current: props.anchor }}
			placement="bottom"
		>
			<div className="p-2 max-h-80 max-w-90 overflow-y-auto">
				<h6 className="leading-6 px-2 pb-1">
					{props.cluster?.points.length ?? 0} nearby events
				</h6>
				{props.cluster?.points.map((point) => {
					const wallClock = formatWallClock(props.startedAtMs, point.offsetMs);
					return (
						<Button
							key={point.id}
							className="w-full justify-start text-left [text-transform:none] gap-2"
							size="sm"
							onPress={() => {
								props.onSeek(point.offsetMs);
								props.onClose();
							}}
						>
							<div
								aria-hidden="true"
								className="w-[9px] h-[9px] flex-none"
								style={{
									backgroundColor: point.color,
									clipPath: markerShape(point.laneId),
								}}
							/>
							<div>
								<p className="text-sm">{point.label}</p>
								<span className="text-muted text-xs leading-5">
									{formatTimelineOffset(point.offsetMs)}
									{wallClock ? ` · ${wallClock}` : ""}
								</span>
							</div>
						</Button>
					);
				})}
			</div>
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
		<section aria-label="Recording event timeline" className="min-w-0">
			<Button
				aria-expanded={expanded}
				aria-controls={contentId}
				className="w-full min-h-6 h-6 px-1.5 py-0 [border:1px_solid_rgba(148,_163,_184,_0.14)] [border-radius:0.75px] [background-color:rgba(148,_163,_184,_0.04)] text-muted [text-transform:none] [justify-content:stretch]"
				style={{
					backgroundColor: "rgba(148, 163, 184, 0.04)",
					color: "var(--color-muted)",
				}}
				size="sm"
				onPress={() => {
					setExpanded((current) => !current);
					setPicker(null);
				}}
			>
				<div className="flex items-center justify-between gap-2 w-full">
					<span className="[font-weight:700] text-xs leading-5">
						Event timeline
					</span>
					<span className="flex items-center">
						<span className="[color:inherit] text-xs leading-5">
							{model.totalEvents} event{model.totalEvents === 1 ? "" : "s"}
							{intervalCount > 0
								? ` · ${intervalCount} period${intervalCount === 1 ? "" : "s"}`
								: ""}
						</span>
						<ExpandMoreIcon
							size={17}
							style={{
								marginLeft: 2,
								transform: expanded ? "rotate(180deg)" : "none",
								transition: "transform 150ms ease",
							}}
						/>
					</span>
				</div>
			</Button>

			{expanded && (
				<div id={contentId}>
					<div
						data-testid="event-timeline-content"
						className="flex min-w-0 mt-1"
					>
						<div aria-hidden="true" className="w-19 min-[600px]:w-26 flex-none">
							{model.lanes.map((lane) => (
								<div
									key={lane.id}
									className="flex items-center justify-end pr-3 min-w-0"
									style={{ height: laneHeight(lane) }}
								>
									<span
										title={lane.label}
										className="text-muted text-xs leading-5 truncate"
									>
										{lane.label}
									</span>
								</div>
							))}
						</div>

						<div
							data-testid="event-timeline-plot"
							ref={plotRef}
							className="relative flex-1 min-w-0"
						>
							{model.lanes.map((lane, laneIndex) => {
								const height = laneHeight(lane);
								const isFirstLane = laneIndex === 0;
								const isLastLane = laneIndex === model.lanes.length - 1;
								return (
									<div
										key={lane.id}
										className={cn(
											"relative",
											laneIndex % 2 === 0
												? "[background-color:rgba(148,_163,_184,_0.11)]"
												: "[background-color:rgba(148,_163,_184,_0.04)]",
											isLastLane
												? "[box-shadow:none]"
												: "[box-shadow:inset_0_-1px_0_rgba(148,_163,_184,_0.14)]",
										)}
										style={{
											height: height,
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
												<TooltipTrigger delay={400} key={interval.id}>
													<Focusable>
														<button
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
																		fraction *
																			(interval.endMs - interval.startMs),
																);
															}}
															className={cn(
																"absolute p-0 overflow-hidden [border:0px_solid] [box-shadow:inset_0_1px_0_rgba(255,255,255,0.35),_0_1px_2px_rgba(2,6,23,0.5)] [color:#0f172a] [cursor:pointer] [font-size:10px] [font-weight:700] [line-height:1] text-left whitespace-nowrap [text-overflow:ellipsis] [z-index:1] hover:[filter:brightness(1.14)] hover:[z-index:4] focus-visible:[filter:brightness(1.14)] focus-visible:[z-index:4]",
																widthPx >= 60 ? "px-1.5" : "px-0",
																interval.startsAtBoundary
																	? "[border-top-left-radius:0px]"
																	: "[border-top-left-radius:999px]",
																interval.startsAtBoundary
																	? "[border-bottom-left-radius:0px]"
																	: "[border-bottom-left-radius:999px]",
																interval.endsAtBoundary
																	? "[border-top-right-radius:0px]"
																	: "[border-top-right-radius:999px]",
																interval.endsAtBoundary
																	? "[border-bottom-right-radius:0px]"
																	: "[border-bottom-right-radius:999px]",
															)}
															style={{
																left: `${left}%`,
																top:
																	LANE_PADDING_PX +
																	interval.track * TRACK_HEIGHT_PX,
																width: `max(3px, ${width}%)`,
																height: TRACK_HEIGHT_PX - 6,
																backgroundColor: interval.color,
															}}
														>
															{widthPx >= 60 ? interval.label : ""}
														</button>
													</Focusable>
													<Tooltip>
														<IntervalTooltip
															interval={interval}
															startedAtMs={props.startedAtMs}
														/>
													</Tooltip>
												</TooltipTrigger>
											);
										})}

										{(clustersByLane.get(lane.id) ?? []).map((cluster) => {
											const clustered = cluster.points.length > 1;
											const markerSize = clustered ? 16 : 11;
											return (
												<TooltipTrigger delay={400} key={cluster.id}>
													<Focusable>
														<button
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
															className={cn(
																"absolute [transform:translate(-50%,_-50%)] p-0 [border:0px_solid] [color:white] [cursor:pointer] [font-size:9px] [font-weight:800] [box-shadow:0_0_0_1.25px_rgba(255,255,255,0.85),_0_1px_3px_rgba(2,6,23,0.7)] [z-index:3] hover:[transform:translate(-50%,_-50%)_scale(1.2)] hover:[z-index:5] focus-visible:[transform:translate(-50%,_-50%)_scale(1.2)] focus-visible:[z-index:5]",
																clustered
																	? "[border-radius:50%]"
																	: "[border-radius:0px]",
															)}
															style={{
																left: `${percent(cluster.offsetMs, props.durationMs)}%`,
																top:
																	LANE_PADDING_PX +
																	cluster.track * TRACK_HEIGHT_PX +
																	TRACK_HEIGHT_PX / 2,
																width: markerSize,
																height: markerSize,
																clipPath: clustered
																	? undefined
																	: markerShape(cluster.laneId),
																backgroundColor: clusterColor(cluster),
																lineHeight: `${markerSize}px`,
															}}
														>
															{clustered ? cluster.points.length : ""}
														</button>
													</Focusable>
													<Tooltip>
														<PointTooltip
															cluster={cluster}
															startedAtMs={props.startedAtMs}
														/>
													</Tooltip>
												</TooltipTrigger>
											);
										})}
									</div>
								);
							})}

							{/* After the lanes so the guides sit above their fills, but below
					    the markers, which own the foreground. */}
							<TimelineGrid />

							{playheadPercent !== null && (
								<TimelinePlayhead percent={playheadPercent} />
							)}
						</div>
					</div>

					<TimelineRow labelAlign="flex-start" className="mt-1">
						<div className="relative h-4.5">
							{TIMELINE_AXIS_FRACTIONS.map((fraction, index) => (
								<div
									key={fraction}
									className={cn(
										"absolute top-0",
										index % 2 === 1 ? "hidden min-[600px]:block" : "block",
									)}
									style={{ left: `${fraction * 100}%` }}
								>
									<div
										aria-hidden="true"
										className="absolute top-0 left-0 w-[1px]"
										style={{
											marginLeft: gridLineOffset(fraction),
											height: AXIS_TICK_HEIGHT_PX,
											backgroundColor: TIMELINE_GRID_COLOR,
										}}
									/>
									<span
										className="text-muted text-xs leading-5 block whitespace-nowrap tabular-nums [line-height:1.2]"
										style={{
											marginTop: `${AXIS_TICK_HEIGHT_PX}px`,
											transform: axisLabelTransform(fraction),
										}}
									>
										{formatTimelineOffset(fraction * props.durationMs)}
									</span>
								</div>
							))}
						</div>
					</TimelineRow>

					<ClusterPicker
						anchor={picker?.anchor ?? null}
						cluster={picker?.cluster ?? null}
						startedAtMs={props.startedAtMs}
						onClose={() => setPicker(null)}
						onSeek={props.onSeek}
					/>
				</div>
			)}
		</section>
	);
}
