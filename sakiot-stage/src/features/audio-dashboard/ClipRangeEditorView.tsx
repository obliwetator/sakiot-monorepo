import {
	Repeat2 as LoopIcon,
	Play as PlayArrowIcon,
	RotateCcw as RestartAltIcon,
	Square as StopIcon,
	ZoomIn as ZoomInIcon,
	ZoomOut as ZoomOutIcon,
} from "lucide-react";
import type { CSSProperties } from "react";
import {
	Badge,
	Button,
	cn,
	IconButton,
	Tooltip,
	TooltipTrigger,
} from "../../shared/ui";
import { formatDuration, formatDurationPrecise } from "../../utils/formatTime";
import { ClipRangePrecisionOverlay } from "./ClipRangePrecisionOverlay";
import type { ClipRangeEditorProps } from "./clipRangeEditorTypes";
import { nudgeEdge, ULTRA_FINE_DRAG_START_PX } from "./clipSelection";
import {
	MAX_CLIP_DURATION_MS,
	MIN_CLIP_DURATION_MS,
} from "./logicalSessionSelection";
import {
	axisLabelTransform,
	gridLineOffset,
	TIMELINE_AXIS_FRACTIONS,
	TIMELINE_GRID_COLOR,
	TIMELINE_PLAYHEAD_COLOR,
	TIMELINE_PLAYHEAD_SHADOW,
	TimelineRow,
} from "./timelineLayout";
import type { ClipRangeViewportController } from "./useClipRangeViewport";
import { WaveformCanvas } from "./WaveformCanvas";

const DETAIL_HEIGHT_PX = 88;
const OVERVIEW_HEIGHT_PX = 16;
const HANDLE_WIDTH_PX = 11;
const ROLLING_EDGE_ZONE_PX = 48;
const FINE_AXIS_FRACTIONS = [0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1];

function percent(fraction: number): string {
	return `${Math.min(1, Math.max(0, fraction)) * 100}%`;
}

function signedSeconds(deltaMs: number): string {
	const seconds = deltaMs / 1_000;
	return `${seconds >= 0 ? "+" : "−"}${Math.abs(seconds).toFixed(1)}s`;
}

export function ClipRangeEditorView({
	props,
	controller,
}: {
	props: ClipRangeEditorProps;
	controller: ClipRangeViewportController;
}) {
	const {
		plotRef,
		dragFeedback,
		viewDragHandlers,
		viewDragging,
		view,
		peaks,
		startFraction,
		endFraction,
		stampFraction,
		selectionGeometry,
		valid,
		dragInvalid,
		selectionDrag,
		dragHandlers,
		onHandleKeyDown,
		suggestedEdge,
		displaySelection,
		playheadFraction,
		fineWindow,
		fineLimitWindow,
		rollingStrength,
		fineSelectionGeometry,
		otherEdgeFraction,
		fineValueFraction,
		fineLimitFraction,
		preciseAxis,
		selectionMs,
		canSetStart,
		canSetEnd,
		zoom,
	} = controller;
	const { durationMs, onSelectionChange, selection } = props;
	return (
		<section aria-label="Clip range editor" className="mb-4">
			<ClipRangePrecisionOverlay controller={controller} />
			<TimelineRow label="Session" className="mb-1">
				<div
					data-testid="clip-session-window"
					{...viewDragHandlers("overview")}
					className={cn(
						"relative rounded-[0.5px] bg-muted/11 touch-none select-none overflow-hidden",
						viewDragging === "overview" ? "cursor-grabbing" : "cursor-grab",
					)}
					style={{ height: OVERVIEW_HEIGHT_PX }}
				>
					<div
						aria-hidden="true"
						className="absolute top-0 bottom-0 bg-accent opacity-55 rounded-[0.5px]"
						style={{
							left: percent(view.startMs / Math.max(1, durationMs)),
							width: `max(3px, ${
								((view.endMs - view.startMs) / Math.max(1, durationMs)) * 100
							}%)`,
						}}
					/>
					<div
						aria-hidden="true"
						className="absolute top-0 bottom-0 w-0.5 -translate-x-px"
						style={{
							left: percent(props.positionMs / Math.max(1, durationMs)),
							backgroundColor: TIMELINE_PLAYHEAD_COLOR,
							boxShadow: TIMELINE_PLAYHEAD_SHADOW,
						}}
					/>
				</div>
			</TimelineRow>

			<TimelineRow label="Clip window" labelAlign="flex-start">
				<div
					ref={plotRef}
					{...viewDragHandlers("detail")}
					className="relative cursor-ew-resize touch-none select-none"
					style={{ height: DETAIL_HEIGHT_PX }}
				>
					<div className="absolute inset-0 rounded-[1px] overflow-hidden bg-purple-500/18">
						<WaveformCanvas
							peaks={peaks}
							height={DETAIL_HEIGHT_PX}
							label="Clip window waveform"
							startFraction={view.startMs / Math.max(1, durationMs)}
							endFraction={view.endMs / Math.max(1, durationMs)}
						/>
						{[
							{ key: "before", left: "0%", right: percent(1 - startFraction) },
							{ key: "after", left: percent(endFraction), right: "0%" },
						].map((mask) => (
							<div
								key={mask.key}
								aria-hidden="true"
								className="absolute top-0 bottom-0 bg-slate-900/60 pointer-events-none"
								style={{ left: mask.left, right: mask.right }}
							/>
						))}
					</div>

					{TIMELINE_AXIS_FRACTIONS.map((fraction) => (
						<div
							key={fraction}
							aria-hidden="true"
							className="absolute top-0 bottom-0 w-[1px] pointer-events-none"
							style={{
								left: percent(fraction),
								marginLeft: gridLineOffset(fraction),
								backgroundColor: TIMELINE_GRID_COLOR,
							}}
						/>
					))}

					{stampFraction !== null &&
						stampFraction >= 0 &&
						stampFraction <= 1 && (
							<div
								aria-hidden="true"
								className="absolute top-0 bottom-0 border-l border-dashed border-amber-300 pointer-events-none z-3"
								style={{ left: percent(stampFraction) }}
							>
								<span className="text-xs leading-5 absolute top-0.5 left-1 text-amber-300 text-shadow-[0_1px_2px_rgba(2,6,23,0.9)]">
									Stamp
								</span>
							</div>
						)}

					{selectionGeometry.overlaps && (
						<button
							{...dragHandlers({ type: "band" })}
							type="button"
							tabIndex={-1}
							aria-label="Move clip selection; click to set nearest edge"
							title="Drag to move the selection, or click to set the nearest edge"
							className={cn(
								"absolute top-0 bottom-0 border-y-2 border-x-2 cursor-grab touch-none active:cursor-grabbing",
								valid && !dragInvalid ? "bg-info/28" : "bg-danger/28",
								selectionDrag.snapshot
									? "[border-top-style:dashed] [border-bottom-style:dashed]"
									: "",
								valid && !dragInvalid ? "border-t-sky-300" : "border-t-red-300",
								valid && !dragInvalid ? "border-b-sky-300" : "border-b-red-300",
								valid && !dragInvalid ? "border-l-sky-300" : "border-l-red-300",
								valid && !dragInvalid ? "border-r-sky-300" : "border-r-red-300",
								selectionDrag.snapshot ? "opacity-70" : "opacity-100",
							)}
							style={{
								left: percent(selectionGeometry.startFraction),
								width: `max(2px, ${
									(selectionGeometry.endFraction -
										selectionGeometry.startFraction) *
									100
								}%)`,
							}}
						/>
					)}

					{(["start", "end"] as const).map((edge) => {
						const fraction = edge === "start" ? startFraction : endFraction;
						const valueMs =
							edge === "start" ? displaySelection[0] : displaySelection[1];
						const handleVisible =
							edge === "start"
								? selectionGeometry.startHandleVisible
								: selectionGeometry.endHandleVisible;
						if (!handleVisible) return null;
						return (
							<div
								key={edge}
								{...dragHandlers({ type: "edge", edge })}
								onKeyDown={(event) => onHandleKeyDown(event, edge)}
								role="slider"
								tabIndex={0}
								aria-label={
									edge === "start" ? "Clip in point" : "Clip out point"
								}
								aria-valuemin={0}
								aria-valuemax={durationMs}
								aria-valuenow={Math.round(valueMs)}
								aria-valuetext={formatDurationPrecise(valueMs / 1_000)}
								className={cn(
									'absolute -top-1 -bottom-1 rounded-[1px] shadow-[0_1px_4px_rgba(2,6,23,0.7)] cursor-ew-resize z-11 outline-offset-2 touch-none grid place-items-center after:content-[""] after:w-[3px] after:h-[40%] after:rounded-[2px] after:bg-slate-900/55 focus-visible:outline-2 focus-visible:outline-(--color-focus) focus-visible:outline-offset-2',
									valid && !dragInvalid ? "bg-sky-300" : "bg-red-300",
									edge === suggestedEdge
										? "outline-2 outline-sky-300/72"
										: "outline-none",
								)}
								style={{
									left: percent(fraction),
									width: HANDLE_WIDTH_PX,
									marginLeft: `${-HANDLE_WIDTH_PX / 2}px`,
								}}
							/>
						);
					})}

					{playheadFraction >= 0 && playheadFraction <= 1 && (
						<div
							{...viewDragHandlers("detail")}
							role="slider"
							tabIndex={-1}
							aria-label="Clip playhead"
							aria-valuemin={Math.round(view.startMs)}
							aria-valuemax={Math.round(view.endMs)}
							aria-valuenow={Math.round(props.positionMs)}
							aria-valuetext={formatDurationPrecise(props.positionMs / 1_000)}
							className={
								'absolute top-0 bottom-0 z-10 cursor-ew-resize touch-none after:content-[""] after:absolute after:top-0 after:bottom-0 after:left-1/2 after:w-0.5 after:-translate-x-px after:bg-(--after-background-color) after:shadow-(--after-box-shadow)'
							}
							style={
								{
									left: percent(playheadFraction),
									width: HANDLE_WIDTH_PX,
									marginLeft: `${-HANDLE_WIDTH_PX / 2}px`,
									"--after-background-color": TIMELINE_PLAYHEAD_COLOR,
									"--after-box-shadow": TIMELINE_PLAYHEAD_SHADOW,
								} as CSSProperties
							}
						/>
					)}

					{fineWindow && fineLimitWindow && dragFeedback && (
						<div
							aria-hidden="true"
							className={
								'absolute top-1 left-1 right-1 z-12 overflow-hidden border border-focus rounded-[1px] bg-slate-900/94 shadow-[0_4px_14px_rgba(2,6,23,0.55)] pointer-events-none before:content-[""] before:absolute before:top-0 before:bottom-0 before:w-14 before:z-5 after:content-[""] after:absolute after:top-0 after:bottom-0 after:w-14 after:z-5 before:left-0 before:bg-linear-to-r before:from-slate-900/96 before:to-slate-900/0 after:right-0 after:bg-linear-to-l after:from-slate-900/96 after:to-slate-900/0'
							}
							style={{ height: DETAIL_HEIGHT_PX / 2 } as CSSProperties}
						>
							{rollingStrength !== 0 && (
								<div
									className={cn(
										"absolute top-0 bottom-0 z-6 grid place-items-center text-focus",
										rollingStrength < 0 ? "left-0" : "left-auto",
										rollingStrength > 0 ? "right-0" : "right-auto",
										rollingStrength < 0
											? "bg-linear-to-r from-info/42 to-info/0"
											: "bg-linear-to-l from-info/42 to-info/0",
									)}
									style={
										{
											width: ROLLING_EDGE_ZONE_PX,
											opacity: 0.45 + Math.abs(rollingStrength) * 0.55,
										} as CSSProperties
									}
								>
									{rollingStrength < 0 ? "←" : "→"}
								</div>
							)}
							{fineSelectionGeometry?.overlaps && (
								<div
									className={cn(
										"absolute top-0 bottom-0 border-y-2 z-1",
										valid ? "bg-info/20" : "bg-danger/20",
										valid ? "border-t-sky-300" : "border-t-red-300",
										valid ? "border-b-sky-300" : "border-b-red-300",
									)}
									style={
										{
											left: percent(fineSelectionGeometry.startFraction),
											width: `${
												(fineSelectionGeometry.endFraction -
													fineSelectionGeometry.startFraction) *
												100
											}%`,
										} as CSSProperties
									}
								/>
							)}
							{FINE_AXIS_FRACTIONS.map((fraction) => (
								<div
									key={fraction}
									className={cn(
										"absolute bottom-0 w-[1px] bg-slate-200/28 z-2",
										fraction === 0 || fraction === 0.5 || fraction === 1
											? "top-5.5"
											: "top-7.5",
									)}
									style={
										{
											left: percent(fraction),
											marginLeft: gridLineOffset(fraction),
										} as CSSProperties
									}
								/>
							))}
							{otherEdgeFraction !== null &&
								otherEdgeFraction >= 0 &&
								otherEdgeFraction <= 1 && (
									<div
										className="absolute top-4.5 bottom-2.5 border-l-2 border-dashed border-amber-300 z-4"
										style={
											{ left: percent(otherEdgeFraction) } as CSSProperties
										}
									>
										<span className="text-xs leading-5 absolute -top-4 left-[3px] text-amber-300">
											{dragFeedback.kind.type === "edge" &&
											dragFeedback.kind.edge === "start"
												? "Out"
												: "In"}
										</span>
									</div>
								)}
							<div
								className="absolute top-4.5 bottom-2.5 w-[3px] -translate-x-px bg-sky-300 z-7"
								style={
									{
										left: percent(fineValueFraction ?? 0),
										boxShadow: TIMELINE_PLAYHEAD_SHADOW,
									} as CSSProperties
								}
							/>
							<Badge
								className="absolute top-1 left-1/2 -translate-x-1/2 tabular-nums z-8"
								size={"sm"}
							>{`${
								fineLimitFraction !== null && fineLimitFraction <= 0
									? "← limit · "
									: fineLimitFraction !== null && fineLimitFraction >= 1
										? "limit → · "
										: ""
							}${
								dragFeedback.kind.type === "playhead"
									? "Head"
									: dragFeedback.kind.type === "band"
										? "Move"
										: dragFeedback.kind.edge === "start"
											? "In"
											: "Out"
							} · ${formatDurationPrecise(
								dragFeedback.valueMs / 1_000,
							)} · ${signedSeconds(
								dragFeedback.valueMs - dragFeedback.originMs,
							)}`}</Badge>
							<span className="text-xs leading-5 absolute top-[5px] left-2 font-bold text-focus z-8">
								{dragFeedback.multiplier >= 100 ? "ULTRA ×100" : "FINE ×10"}
							</span>
							{dragFeedback.multiplier === 10 && (
								<span className="text-xs leading-5 absolute top-[5px] right-2 text-muted z-8">
									↑{" "}
									{Math.max(
										0,
										ULTRA_FINE_DRAG_START_PX - Math.max(0, -dragFeedback.dyPx),
									).toFixed(0)}
									px to ultra
								</span>
							)}
							<span className="text-xs leading-5 absolute left-2 bottom-[3px] tabular-nums z-8">
								Start {formatDurationPrecise(fineLimitWindow.startMs / 1_000)}
							</span>
							<span className="text-xs leading-5 absolute right-2 bottom-[3px] tabular-nums z-8">
								End {formatDurationPrecise(fineLimitWindow.endMs / 1_000)}
							</span>
						</div>
					)}
				</div>
			</TimelineRow>

			<TimelineRow labelAlign="flex-start" className="mt-1">
				<div className="relative h-4.5">
					{TIMELINE_AXIS_FRACTIONS.map((fraction, index) => {
						const atMs = view.startMs + fraction * (view.endMs - view.startMs);
						return (
							<div
								key={fraction}
								className={cn(
									"absolute top-0",
									index % 2 === 1 ? "hidden min-[600px]:block" : "block",
								)}
								style={{ left: percent(fraction) } as CSSProperties}
							>
								<span
									className="text-muted text-xs leading-5 block whitespace-nowrap tabular-nums leading-[1.2]"
									style={
										{ transform: axisLabelTransform(fraction) } as CSSProperties
									}
								>
									{preciseAxis
										? formatDurationPrecise(atMs / 1_000)
										: formatDuration(atMs / 1_000)}
								</span>
							</div>
						);
					})}
				</div>
			</TimelineRow>

			<TimelineRow className="mt-2">
				<div className="flex items-center flex-wrap flex-row gap-2">
					<p className="text-sm tabular-nums">
						In {formatDurationPrecise(selection[0] / 1_000)} · Out{" "}
						{formatDurationPrecise(selection[1] / 1_000)} · Length{" "}
						{(selectionMs / 1_000).toFixed(1)}s
					</p>
					<Badge
						appearance={valid ? "solid" : "outline"}
						tone={valid ? "success" : "neutral"}
						size={"sm"}
					>
						{valid
							? "Valid clip"
							: `Clip needs ${MIN_CLIP_DURATION_MS / 1_000}–${
									MAX_CLIP_DURATION_MS / 1_000
								}s`}
					</Badge>
					<span className="text-muted text-xs leading-5">
						Pull a handle or playhead upward while dragging for a magnified
						ruler. E sets the nearest edge · R resets the selection.
					</span>
					<div className="flex-1" />
					<TooltipTrigger delay={400}>
						<IconButton
							aria-label={"Zoom out (ctrl + scroll)"}
							size="sm"
							onPress={() => zoom(-1)}
						>
							<ZoomOutIcon size={16} />
						</IconButton>
						<Tooltip>{"Zoom out (ctrl + scroll)"}</Tooltip>
					</TooltipTrigger>
					<span className="text-muted text-xs leading-5 tabular-nums">
						{formatDuration((view.endMs - view.startMs) / 1_000)}
					</span>
					<TooltipTrigger delay={400}>
						<IconButton
							aria-label={"Zoom in (ctrl + scroll)"}
							size="sm"
							onPress={() => zoom(1)}
						>
							<ZoomInIcon size={16} />
						</IconButton>
						<Tooltip>{"Zoom in (ctrl + scroll)"}</Tooltip>
					</TooltipTrigger>
				</div>
			</TimelineRow>

			<TimelineRow className="mt-2">
				<div className="flex items-center flex-wrap flex-row gap-2">
					<TooltipTrigger delay={400}>
						<Button
							variant="primary"
							size="sm"
							onPress={props.onSetNearestEdgeFromPlayhead}
						>
							Set nearest: {suggestedEdge === "start" ? "left" : "right"} (E)
						</Button>
						<Tooltip>{`Set the ${suggestedEdge === "start" ? "left" : "right"} edge nearest the playhead (E)`}</Tooltip>
					</TooltipTrigger>
					{(["start", "end"] as const).map((edge) => (
						<div key={edge} className="flex items-center flex-row gap-1">
							<TooltipTrigger delay={400}>
								<Button
									variant="outline"
									size="sm"
									isDisabled={edge === "start" ? !canSetStart : !canSetEnd}
									onPress={() => props.onSetEdgeFromPlayhead(edge)}
								>
									Set {edge === "start" ? "left edge (I)" : "right edge (O)"}
								</Button>
								<Tooltip>
									{(edge === "start" ? canSetStart : canSetEnd)
										? `Set the ${edge === "start" ? "left" : "right"} edge to the playhead (${edge === "start" ? "I" : "O"})`
										: `Move the playhead ${edge === "start" ? "left of the right" : "right of the left"} edge first`}
								</Tooltip>
							</TooltipTrigger>
							{[-1_000, -100, 100, 1_000].map((deltaMs) => (
								<Button
									key={deltaMs}
									className="min-w-11 px-1"
									variant="ghost"
									size="sm"
									onPress={() =>
										onSelectionChange(
											nudgeEdge(selection, edge, deltaMs, durationMs),
										)
									}
								>
									{deltaMs > 0 ? "+" : "−"}
									{Math.abs(deltaMs) / 1_000}s
								</Button>
							))}
						</div>
					))}
					{props.edgeHint && (
						<span className="text-warning text-xs leading-5 basis-full">
							{props.edgeHint}
						</span>
					)}
					<div className="flex-1" />
					<TooltipTrigger delay={400}>
						<Button variant="outline" size="sm" onPress={props.onReset}>
							<RestartAltIcon />
							Reset
						</Button>
						<Tooltip>{"Reset clip selection (R)"}</Tooltip>
					</TooltipTrigger>
					<Button variant="primary" size="sm" onPress={props.onPreview}>
						{props.previewing ? <StopIcon /> : <PlayArrowIcon />}
						{props.previewing ? "Stop" : "Preview"}
					</Button>
					<Button
						variant={props.loop ? "primary" : "outline"}
						aria-pressed={props.loop}
						size="sm"
						onPress={() => props.onLoopChange(!props.loop)}
					>
						<LoopIcon />
						Loop
					</Button>
				</div>
			</TimelineRow>
		</section>
	);
}
