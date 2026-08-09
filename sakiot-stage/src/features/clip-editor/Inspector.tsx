import ContentCutIcon from "@mui/icons-material/ContentCut";
import DeleteIcon from "@mui/icons-material/Delete";
import ReplayIcon from "@mui/icons-material/Replay";
import Box from "@mui/material/Box";
import MuiButton, { type ButtonProps } from "@mui/material/Button";
import Divider from "@mui/material/Divider";
import Slider from "@mui/material/Slider";
import Stack from "@mui/material/Stack";
import Tooltip from "@mui/material/Tooltip";
import Typography from "@mui/material/Typography";
import { formatDuration } from "../../utils/formatTime";
import {
	type InspectorFeatureId,
	isInspectorFeatureDisabled,
	MULTI_SELECTION_DISABLED_REASON,
} from "./inspectorFeaturePolicy";
import {
	resizeSelectedSegments,
	type SegmentEffects,
	segmentDuration,
	type TimelineSegment,
} from "./model";
import type { UseClipEditorReturn } from "./useClipEditor";

const SLIDER_PROPS = {
	size: "small" as const,
	sx: { "& .MuiSlider-thumb, & .MuiSlider-track": { transition: "none" } },
};

export function Inspector(props: {
	editor: UseClipEditorReturn;
	clipName: (sourceId: string) => string;
}) {
	const { editor } = props;
	const segment = editor.selectedSegment;

	return (
		<Box
			sx={{
				width: 260,
				flex: "0 0 auto",
				minHeight: 0,
				minWidth: 0,
				overflowY: "auto",
				overflowX: "hidden",
				borderLeft: 1,
				borderColor: "divider",
				p: 2,
			}}
		>
			{segment ? (
				<SegmentInspectorContent
					editor={editor}
					segment={segment}
					clipName={props.clipName}
				/>
			) : (
				<>
					<Typography variant="overline" color="text.secondary">
						Inspector
					</Typography>
					<Typography variant="body2" color="text.secondary" sx={{ mt: 1 }}>
						No segment selected.
					</Typography>
					<Typography
						variant="caption"
						color="text.secondary"
						display="block"
						sx={{ mt: 0.5 }}
					>
						Click a clip on the timeline to trim it and adjust its effects.
					</Typography>
				</>
			)}
		</Box>
	);
}

function SegmentInspectorContent(props: {
	editor: UseClipEditorReturn;
	segment: TimelineSegment;
	clipName: (sourceId: string) => string;
}) {
	const { editor } = props;
	const segment = props.segment;
	const segments = editor.selectedSegments;
	const multi = segments.length > 1;
	const ids = segments.map((s) => s.id);

	const patchEffects = (
		feature: InspectorFeatureId,
		patch: Partial<SegmentEffects>,
	) => {
		if (isInspectorFeatureDisabled(feature, segments.length)) return;
		editor.preview((edit) => ({
			...edit,
			segments: edit.segments.map((s) =>
				ids.includes(s.id) ? { ...s, effects: { ...s.effects, ...patch } } : s,
			),
		}));
	};

	// Speed and pitch resize the boxes; the resize runs over the selected
	// group so snapped members move together while others behave normally.
	const patchResizingEffect = (
		feature: InspectorFeatureId,
		key: "rate" | "pitchCents",
		value: number,
	) => {
		if (isInspectorFeatureDisabled(feature, segments.length)) return;
		editor.preview((edit) =>
			resizeSelectedSegments(edit, ids, (_id, effects) => ({
				...effects,
				[key]: value,
			})),
		);
	};

	const finishSlider = () => editor.flush();

	const duration = segmentDuration(segment);

	return (
		<>
			<Typography variant="overline" color="text.secondary">
				{multi ? `${segments.length} segments selected` : "Selected segment"}
			</Typography>
			<Typography
				variant="h6"
				noWrap
				title={
					multi
						? segments.map((s) => props.clipName(s.sourceId)).join(", ")
						: props.clipName(segment.sourceId)
				}
			>
				{props.clipName(segment.sourceId)}
				{multi ? ` +${segments.length - 1}` : ""}
			</Typography>
			<Typography variant="caption" color="text.secondary">
				{multi
					? "Effect changes apply to all selected segments."
					: `${formatDuration(duration)} · at ${formatDuration(segment.timelineStart)}`}
			</Typography>

			<Divider sx={{ my: 2 }} />

			<Typography variant="overline" color="text.secondary">
				Effects
			</Typography>

			{multi && (
				<Box sx={{ mt: 1 }}>
					<Typography variant="body2" color="text.secondary">
						Editing effects for several segments at once can shift their boxes
						unexpectedly.
					</Typography>
				</Box>
			)}
			<EffectSlider
				feature="volume"
				selectionCount={segments.length}
				label="Volume"
				value={segment.effects.volumeDb}
				min={-40}
				max={12}
				step={0.5}
				format={(value) => `${value.toFixed(1)} dB`}
				onChange={(value) => patchEffects("volume", { volumeDb: value })}
				onCommitted={finishSlider}
			/>
			<EffectSlider
				feature="pitch"
				selectionCount={segments.length}
				label="Pitch"
				value={segment.effects.pitchCents}
				min={-1200}
				max={1200}
				step={10}
				format={(value) =>
					value === 0 ? "0" : `${value > 0 ? "+" : ""}${value} ct`
				}
				onChange={(value) => patchResizingEffect("pitch", "pitchCents", value)}
				onCommitted={finishSlider}
			/>
			<EffectSlider
				feature="speed"
				selectionCount={segments.length}
				label="Speed"
				value={segment.effects.rate}
				min={0.5}
				max={2}
				step={0.05}
				format={(value) => `${value.toFixed(2)}×`}
				onChange={(value) => patchResizingEffect("speed", "rate", value)}
				onCommitted={finishSlider}
			/>
			<EffectSlider
				feature="bass"
				selectionCount={segments.length}
				label="Bass"
				value={segment.effects.bassDb}
				min={-12}
				max={12}
				step={0.5}
				format={(value) => `${value > 0 ? "+" : ""}${value.toFixed(1)} dB`}
				onChange={(value) => patchEffects("bass", { bassDb: value })}
				onCommitted={finishSlider}
			/>
			<EffectSlider
				feature="treble"
				selectionCount={segments.length}
				label="Treble"
				value={segment.effects.trebleDb}
				min={-12}
				max={12}
				step={0.5}
				format={(value) => `${value > 0 ? "+" : ""}${value.toFixed(1)} dB`}
				onChange={(value) => patchEffects("treble", { trebleDb: value })}
				onCommitted={finishSlider}
			/>

			<Divider sx={{ my: 2 }} />

			<Stack direction="row" spacing={1} sx={{ flexWrap: "wrap", gap: 0.5 }}>
				<InspectorActionButton
					feature="split"
					selectionCount={segments.length}
					size="small"
					variant="outlined"
					startIcon={<ContentCutIcon />}
					onClick={editor.splitSelectedAtPlayhead}
				>
					Split (S)
				</InspectorActionButton>
				<InspectorActionButton
					feature="reverse"
					selectionCount={segments.length}
					size="small"
					variant={segment.effects.reverse ? "contained" : "outlined"}
					color={segment.effects.reverse ? "secondary" : "primary"}
					aria-pressed={segment.effects.reverse}
					startIcon={<ReplayIcon />}
					onClick={editor.toggleReverse}
				>
					Reverse (R)
				</InspectorActionButton>
				<InspectorActionButton
					feature="delete"
					selectionCount={segments.length}
					size="small"
					variant="outlined"
					color="error"
					startIcon={<DeleteIcon />}
					onClick={editor.removeSelected}
				>
					Delete
				</InspectorActionButton>
			</Stack>
		</>
	);
}

function EffectSlider(props: {
	feature: InspectorFeatureId;
	selectionCount: number;
	label: string;
	value: number;
	min: number;
	max: number;
	step: number;
	format: (value: number) => string;
	onChange: (value: number) => void;
	onCommitted: () => void;
	disabled?: boolean;
}) {
	const gated = isInspectorFeatureDisabled(props.feature, props.selectionCount);
	const disabled = props.disabled || gated;

	return (
		<Tooltip title={gated ? MULTI_SELECTION_DISABLED_REASON : ""}>
			<Box sx={{ mb: 1 }}>
				<Typography
					variant="caption"
					color={disabled ? "text.disabled" : "text.secondary"}
				>
					{props.label} · {props.format(props.value)}
				</Typography>
				<Slider
					{...SLIDER_PROPS}
					min={props.min}
					max={props.max}
					step={props.step}
					value={props.value}
					disabled={disabled}
					onChange={(_event, value) => props.onChange(Number(value))}
					onChangeCommitted={props.onCommitted}
					aria-label={props.label}
				/>
			</Box>
		</Tooltip>
	);
}

function InspectorActionButton(
	props: ButtonProps & {
		feature: InspectorFeatureId;
		selectionCount: number;
	},
) {
	const { feature, selectionCount, disabled, ...buttonProps } = props;
	const gated = isInspectorFeatureDisabled(feature, selectionCount);

	return (
		<Tooltip title={gated ? MULTI_SELECTION_DISABLED_REASON : ""}>
			<span>
				<MuiButton {...buttonProps} disabled={disabled || gated} />
			</span>
		</Tooltip>
	);
}
