import {
	Repeat2 as LoopIcon,
	Play as PlayArrowIcon,
	RotateCcw as RestartAltIcon,
	Square as StopIcon,
	ZoomIn as ZoomInIcon,
	ZoomOut as ZoomOutIcon,
} from "lucide-react";
import {
	Box,
	Button,
	Chip,
	IconButton,
	Stack,
	Tooltip,
	Typography,
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
		<Box component="section" aria-label="Clip range editor" sx={{ mb: 2 }}>
			<ClipRangePrecisionOverlay controller={controller} />
			<TimelineRow label="Session" sx={{ mb: 0.5 }}>
				<Box
					data-testid="clip-session-window"
					{...viewDragHandlers("overview")}
					sx={{
						position: "relative",
						height: OVERVIEW_HEIGHT_PX,
						borderRadius: 0.5,
						bgcolor: "rgba(148, 163, 184, 0.11)",
						cursor: viewDragging === "overview" ? "grabbing" : "grab",
						touchAction: "none",
						userSelect: "none",
						overflow: "hidden",
					}}
				>
					<Box
						aria-hidden="true"
						sx={{
							position: "absolute",
							top: 0,
							bottom: 0,
							left: percent(view.startMs / Math.max(1, durationMs)),
							width: `max(3px, ${
								((view.endMs - view.startMs) / Math.max(1, durationMs)) * 100
							}%)`,
							bgcolor: "primary.main",
							opacity: 0.55,
							borderRadius: 0.5,
						}}
					/>
					<Box
						aria-hidden="true"
						sx={{
							position: "absolute",
							top: 0,
							bottom: 0,
							left: percent(props.positionMs / Math.max(1, durationMs)),
							width: 2,
							transform: "translateX(-1px)",
							bgcolor: TIMELINE_PLAYHEAD_COLOR,
							boxShadow: TIMELINE_PLAYHEAD_SHADOW,
						}}
					/>
				</Box>
			</TimelineRow>

			<TimelineRow label="Clip window" labelAlign="flex-start">
				<Box
					ref={plotRef}
					{...viewDragHandlers("detail")}
					sx={{
						position: "relative",
						height: DETAIL_HEIGHT_PX,
						cursor: "ew-resize",
						touchAction: "none",
						userSelect: "none",
					}}
				>
					<Box
						sx={{
							position: "absolute",
							inset: 0,
							borderRadius: 1,
							overflow: "hidden",
							bgcolor: "rgba(168, 85, 247, 0.18)",
						}}
					>
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
							<Box
								key={mask.key}
								aria-hidden="true"
								sx={{
									position: "absolute",
									top: 0,
									bottom: 0,
									left: mask.left,
									right: mask.right,
									bgcolor: "rgba(2, 6, 23, 0.6)",
									pointerEvents: "none",
								}}
							/>
						))}
					</Box>

					{TIMELINE_AXIS_FRACTIONS.map((fraction) => (
						<Box
							key={fraction}
							aria-hidden="true"
							sx={{
								position: "absolute",
								top: 0,
								bottom: 0,
								left: percent(fraction),
								ml: gridLineOffset(fraction),
								width: "1px",
								bgcolor: TIMELINE_GRID_COLOR,
								pointerEvents: "none",
							}}
						/>
					))}

					{stampFraction !== null &&
						stampFraction >= 0 &&
						stampFraction <= 1 && (
							<Box
								aria-hidden="true"
								sx={{
									position: "absolute",
									top: 0,
									bottom: 0,
									left: percent(stampFraction),
									borderLeftStyle: "dashed",
									borderLeftWidth: 1,
									borderLeftColor: "warning.light",
									pointerEvents: "none",
									zIndex: 3,
								}}
							>
								<Typography
									variant="caption"
									sx={{
										position: "absolute",
										top: 2,
										left: 4,
										color: "warning.light",
										textShadow: "0 1px 2px rgba(2, 6, 23, 0.9)",
									}}
								>
									Stamp
								</Typography>
							</Box>
						)}

					{selectionGeometry.overlaps && (
						<Box
							{...dragHandlers({ type: "band" })}
							role="button"
							tabIndex={-1}
							aria-label="Move clip selection; click to set nearest edge"
							title="Drag to move the selection, or click to set the nearest edge"
							sx={{
								position: "absolute",
								top: 0,
								bottom: 0,
								left: percent(selectionGeometry.startFraction),
								width: `max(2px, ${
									(selectionGeometry.endFraction -
										selectionGeometry.startFraction) *
									100
								}%)`,
								bgcolor:
									valid && !dragInvalid
										? "rgba(56, 189, 248, 0.28)"
										: "rgba(248, 113, 113, 0.28)",
								borderTopStyle: selectionDrag.snapshot ? "dashed" : "solid",
								borderTopWidth: 2,
								borderTopColor:
									valid && !dragInvalid ? "info.light" : "error.light",
								borderBottomStyle: selectionDrag.snapshot ? "dashed" : "solid",
								borderBottomWidth: 2,
								borderBottomColor:
									valid && !dragInvalid ? "info.light" : "error.light",
								borderLeftStyle: "solid",
								borderLeftWidth: 2,
								borderLeftColor:
									valid && !dragInvalid ? "info.light" : "error.light",
								borderRightStyle: "solid",
								borderRightWidth: 2,
								borderRightColor:
									valid && !dragInvalid ? "info.light" : "error.light",
								opacity: selectionDrag.snapshot ? 0.7 : 1,
								cursor: "grab",
								touchAction: "none",
								"&:active": { cursor: "grabbing" },
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
							<Box
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
								sx={{
									position: "absolute",
									top: -4,
									bottom: -4,
									left: percent(fraction),
									width: HANDLE_WIDTH_PX,
									ml: `${-HANDLE_WIDTH_PX / 2}px`,
									borderRadius: 1,
									bgcolor: valid && !dragInvalid ? "info.light" : "error.light",
									boxShadow: "0 1px 4px rgba(2,6,23,0.7)",
									cursor: "ew-resize",
									zIndex: 11,
									outline:
										edge === suggestedEdge
											? "2px solid rgba(125, 211, 252, 0.72)"
											: "none",
									outlineOffset: 2,
									touchAction: "none",
									display: "grid",
									placeItems: "center",
									"&::after": {
										content: '""',
										width: "3px",
										height: "40%",
										borderRadius: "2px",
										bgcolor: "rgba(2, 6, 23, 0.55)",
									},
									"&:focus-visible": {
										outline: "2px solid",
										outlineColor: "primary.light",
										outlineOffset: 2,
									},
								}}
							/>
						);
					})}

					{playheadFraction >= 0 && playheadFraction <= 1 && (
						<Box
							{...viewDragHandlers("detail")}
							role="slider"
							tabIndex={-1}
							aria-label="Clip playhead"
							aria-valuemin={Math.round(view.startMs)}
							aria-valuemax={Math.round(view.endMs)}
							aria-valuenow={Math.round(props.positionMs)}
							aria-valuetext={formatDurationPrecise(props.positionMs / 1_000)}
							sx={{
								position: "absolute",
								top: 0,
								bottom: 0,
								left: percent(playheadFraction),
								width: HANDLE_WIDTH_PX,
								ml: `${-HANDLE_WIDTH_PX / 2}px`,
								zIndex: 10,
								cursor: "ew-resize",
								touchAction: "none",
								"&::after": {
									content: '""',
									position: "absolute",
									top: 0,
									bottom: 0,
									left: "50%",
									width: 2,
									transform: "translateX(-1px)",
									bgcolor: TIMELINE_PLAYHEAD_COLOR,
									boxShadow: TIMELINE_PLAYHEAD_SHADOW,
								},
							}}
						/>
					)}

					{fineWindow && fineLimitWindow && dragFeedback && (
						<Box
							aria-hidden="true"
							sx={{
								position: "absolute",
								top: 4,
								height: DETAIL_HEIGHT_PX / 2,
								left: 4,
								right: 4,
								zIndex: 12,
								overflow: "hidden",
								border: "1px solid",
								borderColor: "primary.light",
								borderRadius: 1,
								bgcolor: "rgba(2, 6, 23, 0.94)",
								boxShadow: "0 4px 14px rgba(2, 6, 23, 0.55)",
								pointerEvents: "none",
								"&::before, &::after": {
									content: '""',
									position: "absolute",
									top: 0,
									bottom: 0,
									width: 56,
									zIndex: 5,
								},
								"&::before": {
									left: 0,
									background:
										"linear-gradient(90deg, rgba(2, 6, 23, 0.96), rgba(2, 6, 23, 0))",
								},
								"&::after": {
									right: 0,
									background:
										"linear-gradient(270deg, rgba(2, 6, 23, 0.96), rgba(2, 6, 23, 0))",
								},
							}}
						>
							{rollingStrength !== 0 && (
								<Box
									sx={{
										position: "absolute",
										top: 0,
										bottom: 0,
										left: rollingStrength < 0 ? 0 : "auto",
										right: rollingStrength > 0 ? 0 : "auto",
										width: ROLLING_EDGE_ZONE_PX,
										zIndex: 6,
										display: "grid",
										placeItems: "center",
										color: "primary.light",
										opacity: 0.45 + Math.abs(rollingStrength) * 0.55,
										background:
											rollingStrength < 0
												? "linear-gradient(90deg, rgba(56, 189, 248, 0.42), rgba(56, 189, 248, 0))"
												: "linear-gradient(270deg, rgba(56, 189, 248, 0.42), rgba(56, 189, 248, 0))",
									}}
								>
									{rollingStrength < 0 ? "←" : "→"}
								</Box>
							)}
							{fineSelectionGeometry?.overlaps && (
								<Box
									sx={{
										position: "absolute",
										top: 0,
										bottom: 0,
										left: percent(fineSelectionGeometry.startFraction),
										width: `${
											(fineSelectionGeometry.endFraction -
												fineSelectionGeometry.startFraction) *
											100
										}%`,
										bgcolor: valid
											? "rgba(56, 189, 248, 0.2)"
											: "rgba(248, 113, 113, 0.2)",
										borderTopStyle: "solid",
										borderTopWidth: 2,
										borderTopColor: valid ? "info.light" : "error.light",
										borderBottomStyle: "solid",
										borderBottomWidth: 2,
										borderBottomColor: valid ? "info.light" : "error.light",
										zIndex: 1,
									}}
								/>
							)}
							{FINE_AXIS_FRACTIONS.map((fraction) => (
								<Box
									key={fraction}
									sx={{
										position: "absolute",
										top:
											fraction === 0 || fraction === 0.5 || fraction === 1
												? 22
												: 30,
										bottom: 0,
										left: percent(fraction),
										ml: gridLineOffset(fraction),
										width: "1px",
										bgcolor: "rgba(226, 232, 240, 0.28)",
										zIndex: 2,
									}}
								/>
							))}
							{otherEdgeFraction !== null &&
								otherEdgeFraction >= 0 &&
								otherEdgeFraction <= 1 && (
									<Box
										sx={{
											position: "absolute",
											top: 18,
											bottom: 10,
											left: percent(otherEdgeFraction),
											borderLeftStyle: "dashed",
											borderLeftWidth: 2,
											borderLeftColor: "warning.light",
											zIndex: 4,
										}}
									>
										<Typography
											variant="caption"
											sx={{
												position: "absolute",
												top: -16,
												left: 3,
												color: "warning.light",
											}}
										>
											{dragFeedback.kind.type === "edge" &&
											dragFeedback.kind.edge === "start"
												? "Out"
												: "In"}
										</Typography>
									</Box>
								)}
							<Box
								sx={{
									position: "absolute",
									top: 18,
									bottom: 10,
									left: percent(fineValueFraction ?? 0),
									width: 3,
									transform: "translateX(-1px)",
									bgcolor: "info.light",
									boxShadow: TIMELINE_PLAYHEAD_SHADOW,
									zIndex: 7,
								}}
							/>
							<Chip
								size="small"
								label={`${
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
								)}`}
								sx={{
									position: "absolute",
									top: 4,
									left: "50%",
									transform: "translateX(-50%)",
									fontVariantNumeric: "tabular-nums",
									zIndex: 8,
								}}
							/>
							<Typography
								variant="caption"
								sx={{
									position: "absolute",
									top: 5,
									left: 8,
									fontWeight: 700,
									color: "primary.light",
									zIndex: 8,
								}}
							>
								{dragFeedback.multiplier >= 100 ? "ULTRA ×100" : "FINE ×10"}
							</Typography>
							{dragFeedback.multiplier === 10 && (
								<Typography
									variant="caption"
									sx={{
										position: "absolute",
										top: 5,
										right: 8,
										color: "text.secondary",
										zIndex: 8,
									}}
								>
									↑{" "}
									{Math.max(
										0,
										ULTRA_FINE_DRAG_START_PX - Math.max(0, -dragFeedback.dyPx),
									).toFixed(0)}
									px to ultra
								</Typography>
							)}
							<Typography
								variant="caption"
								sx={{
									position: "absolute",
									left: 8,
									bottom: 3,
									fontVariantNumeric: "tabular-nums",
									zIndex: 8,
								}}
							>
								Start {formatDurationPrecise(fineLimitWindow.startMs / 1_000)}
							</Typography>
							<Typography
								variant="caption"
								sx={{
									position: "absolute",
									right: 8,
									bottom: 3,
									fontVariantNumeric: "tabular-nums",
									zIndex: 8,
								}}
							>
								End {formatDurationPrecise(fineLimitWindow.endMs / 1_000)}
							</Typography>
						</Box>
					)}
				</Box>
			</TimelineRow>

			<TimelineRow sx={{ mt: 0.5 }} labelAlign="flex-start">
				<Box sx={{ position: "relative", height: 18 }}>
					{TIMELINE_AXIS_FRACTIONS.map((fraction, index) => {
						const atMs = view.startMs + fraction * (view.endMs - view.startMs);
						return (
							<Box
								key={fraction}
								sx={{
									position: "absolute",
									top: 0,
									left: percent(fraction),
									display:
										index % 2 === 1 ? { xs: "none", sm: "block" } : "block",
								}}
							>
								<Typography
									variant="caption"
									color="text.secondary"
									sx={{
										display: "block",
										transform: axisLabelTransform(fraction),
										whiteSpace: "nowrap",
										fontVariantNumeric: "tabular-nums",
										lineHeight: 1.2,
									}}
								>
									{preciseAxis
										? formatDurationPrecise(atMs / 1_000)
										: formatDuration(atMs / 1_000)}
								</Typography>
							</Box>
						);
					})}
				</Box>
			</TimelineRow>

			<TimelineRow sx={{ mt: 1 }}>
				<Stack
					direction="row"
					spacing={1}
					alignItems="center"
					flexWrap="wrap"
					useFlexGap
				>
					<Typography
						variant="body2"
						sx={{ fontVariantNumeric: "tabular-nums" }}
					>
						In {formatDurationPrecise(selection[0] / 1_000)} · Out{" "}
						{formatDurationPrecise(selection[1] / 1_000)} · Length{" "}
						{(selectionMs / 1_000).toFixed(1)}s
					</Typography>
					<Chip
						size="small"
						color={valid ? "success" : "default"}
						variant={valid ? "filled" : "outlined"}
						label={
							valid
								? "Valid clip"
								: `Clip needs ${MIN_CLIP_DURATION_MS / 1_000}–${
										MAX_CLIP_DURATION_MS / 1_000
									}s`
						}
					/>
					<Typography variant="caption" color="text.secondary">
						Pull a handle or playhead upward while dragging for a magnified
						ruler. E sets the nearest edge · R resets the selection.
					</Typography>
					<Box sx={{ flex: 1 }} />
					<Tooltip title="Zoom out (ctrl + scroll)">
						<IconButton size="small" onClick={() => zoom(-1)}>
							<ZoomOutIcon size={16} />
						</IconButton>
					</Tooltip>
					<Typography
						variant="caption"
						color="text.secondary"
						sx={{ fontVariantNumeric: "tabular-nums" }}
					>
						{formatDuration((view.endMs - view.startMs) / 1_000)}
					</Typography>
					<Tooltip title="Zoom in (ctrl + scroll)">
						<IconButton size="small" onClick={() => zoom(1)}>
							<ZoomInIcon size={16} />
						</IconButton>
					</Tooltip>
				</Stack>
			</TimelineRow>

			<TimelineRow sx={{ mt: 1 }}>
				<Stack
					direction="row"
					spacing={1}
					alignItems="center"
					flexWrap="wrap"
					useFlexGap
				>
					<Tooltip
						title={`Set the ${suggestedEdge === "start" ? "left" : "right"} edge nearest the playhead (E)`}
					>
						<Button
							size="small"
							variant="contained"
							onClick={props.onSetNearestEdgeFromPlayhead}
						>
							Set nearest: {suggestedEdge === "start" ? "left" : "right"} (E)
						</Button>
					</Tooltip>
					{(["start", "end"] as const).map((edge) => (
						<Stack key={edge} direction="row" spacing={0.5} alignItems="center">
							<Tooltip
								title={
									(edge === "start" ? canSetStart : canSetEnd)
										? `Set the ${edge === "start" ? "left" : "right"} edge to the playhead (${edge === "start" ? "I" : "O"})`
										: `Move the playhead ${edge === "start" ? "left of the right" : "right of the left"} edge first`
								}
							>
								<span>
									<Button
										size="small"
										variant="outlined"
										disabled={edge === "start" ? !canSetStart : !canSetEnd}
										onClick={() => props.onSetEdgeFromPlayhead(edge)}
									>
										Set {edge === "start" ? "left edge (I)" : "right edge (O)"}
									</Button>
								</span>
							</Tooltip>
							{[-1_000, -100, 100, 1_000].map((deltaMs) => (
								<Button
									key={deltaMs}
									size="small"
									variant="text"
									sx={{ minWidth: 44, px: 0.5 }}
									onClick={() =>
										onSelectionChange(
											nudgeEdge(selection, edge, deltaMs, durationMs),
										)
									}
								>
									{deltaMs > 0 ? "+" : "−"}
									{Math.abs(deltaMs) / 1_000}s
								</Button>
							))}
						</Stack>
					))}
					{props.edgeHint && (
						<Typography
							variant="caption"
							color="warning.main"
							sx={{ flexBasis: "100%" }}
						>
							{props.edgeHint}
						</Typography>
					)}
					<Box sx={{ flex: 1 }} />
					<Tooltip title="Reset clip selection (R)">
						<Button
							size="small"
							variant="outlined"
							startIcon={<RestartAltIcon />}
							onClick={props.onReset}
						>
							Reset
						</Button>
					</Tooltip>
					<Button
						size="small"
						variant="contained"
						startIcon={props.previewing ? <StopIcon /> : <PlayArrowIcon />}
						onClick={props.onPreview}
					>
						{props.previewing ? "Stop" : "Preview"}
					</Button>
					<Button
						size="small"
						variant={props.loop ? "contained" : "outlined"}
						color={props.loop ? "secondary" : "primary"}
						startIcon={<LoopIcon />}
						aria-pressed={props.loop}
						onClick={() => props.onLoopChange(!props.loop)}
					>
						Loop
					</Button>
				</Stack>
			</TimelineRow>
		</Box>
	);
}
