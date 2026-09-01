import {
	GitBranch as CallSplitIcon,
	Scissors as ContentCutIcon,
	Trash2 as DeleteIcon,
	ChevronDown as ExpandMoreIcon,
	GitMerge as MergeIcon,
	RotateCcw as ReplayIcon,
	Settings as SettingsIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import {
	Accordion,
	AccordionDetails,
	AccordionSummary,
	Box,
	type ButtonProps,
	Divider,
	FormControlLabel,
	IconButton,
	Button as InspectorButton,
	Slider,
	Stack,
	Switch,
	LegacyTextField as TextField,
	Tooltip,
	Typography,
} from "../../shared/ui";
import { formatDuration } from "../../utils/formatTime";
import { type EffectLimits, effectLimit } from "./effectLimits";
import {
	type InspectorFeatureId,
	isInspectorFeatureDisabled,
	MULTI_SELECTION_DISABLED_REASON,
} from "./inspectorFeaturePolicy";
import {
	isSingleMergedUnit,
	resizeSelectedSegments,
	type SegmentEffects,
	segmentDuration,
	type TimelineSegment,
} from "./model";
import type { UseClipEditorReturn } from "./useClipEditor";

const SLIDER_PROPS = {
	size: "small" as const,
	sx: { transition: "none" },
};

export function Inspector(props: {
	editor: UseClipEditorReturn;
	clipName: (sourceId: string) => string;
	limits: EffectLimits;
	onOpenLimits: () => void;
}) {
	const { editor } = props;
	const segment = editor.selectedSegment;

	return (
		<Box
			component="aside"
			aria-label="Inspector"
			data-testid="clip-inspector"
			sx={{
				width: { xs: "100%", md: 260 },
				maxHeight: { xs: segment ? "33.333%" : "none", md: "none" },
				flex: {
					xs: segment ? "0 0 33.333%" : "0 0 auto",
					md: "0 0 auto",
				},
				minHeight: 0,
				minWidth: 0,
				overflowY: "auto",
				overflowX: "hidden",
				borderLeft: { xs: 0, md: 1 },
				borderTop: { xs: 1, md: 0 },
				borderColor: "divider",
				p: { xs: 1, md: 2 },
			}}
		>
			{segment ? (
				<SegmentInspectorContent
					editor={editor}
					segment={segment}
					clipName={props.clipName}
					limits={props.limits}
					onOpenLimits={props.onOpenLimits}
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
	limits: EffectLimits;
	onOpenLimits: () => void;
}) {
	const { editor } = props;
	const segment = props.segment;
	const segments = editor.selectedSegments;
	const multi = segments.length > 1;
	const ids = segments.map((s) => s.id);
	const mergedUnit = segments.some((s) => s.mergeGroup);
	const alreadyMerged = isSingleMergedUnit(segments);

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
	const commitEffects = (
		feature: InspectorFeatureId,
		patch: Partial<SegmentEffects>,
	) => {
		if (isInspectorFeatureDisabled(feature, segments.length)) return;
		editor.apply((edit) => ({
			...edit,
			segments: edit.segments.map((s) =>
				ids.includes(s.id) ? { ...s, effects: { ...s.effects, ...patch } } : s,
			),
		}));
	};

	// Speed resizes the boxes; the resize runs over the selected group so
	// snapped members move together while others behave normally. Pitch is a
	// duration-preserving effect and goes through patchEffects instead.
	const patchResizingEffect = (
		feature: InspectorFeatureId,
		patch: Partial<SegmentEffects>,
	) => {
		if (isInspectorFeatureDisabled(feature, segments.length)) return;
		editor.preview((edit) =>
			resizeSelectedSegments(edit, ids, (_id, effects) => ({
				...effects,
				...patch,
			})),
		);
	};

	const finishSlider = () => editor.flush();

	const duration = segmentDuration(segment);

	return (
		<>
			<Stack direction="row" alignItems="center" justifyContent="space-between">
				<Typography variant="overline" color="text.secondary">
					{multi ? `${segments.length} segments selected` : "Selected segment"}
				</Typography>
				<Tooltip title="Adjust the effect limits (volume, pitch, speed, EQ)">
					<IconButton size="small" onClick={props.onOpenLimits}>
						<SettingsIcon size={16} />
					</IconButton>
				</Tooltip>
			</Stack>
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
			{mergedUnit && (
				<Typography variant="caption" color="text.secondary" display="block">
					Merged unit: the clips act as one element. Ungroup to edit them
					individually.
				</Typography>
			)}

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
				min={effectLimit("volumeDb", props.limits)[0]}
				max={effectLimit("volumeDb", props.limits)[1]}
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
				min={effectLimit("pitchCents", props.limits)[0]}
				max={effectLimit("pitchCents", props.limits)[1]}
				step={10}
				format={(value) =>
					value === 0 ? "0" : `${value > 0 ? "+" : ""}${value} ct`
				}
				onChange={(value) => patchEffects("pitch", { pitchCents: value })}
				onCommitted={finishSlider}
			/>
			<EffectSlider
				feature="speed"
				selectionCount={segments.length}
				label="Speed"
				value={segment.effects.rate}
				min={effectLimit("rate", props.limits)[0]}
				max={effectLimit("rate", props.limits)[1]}
				step={0.05}
				format={(value) => `${value.toFixed(2)}×`}
				onChange={(value) => patchResizingEffect("speed", { rate: value })}
				onCommitted={finishSlider}
			/>
			<EffectSlider
				feature="bass"
				selectionCount={segments.length}
				label="Bass"
				value={segment.effects.bassDb}
				min={effectLimit("bassDb", props.limits)[0]}
				max={effectLimit("bassDb", props.limits)[1]}
				step={0.5}
				format={(value) => `${value > 0 ? "+" : ""}${value.toFixed(1)} dB`}
				onChange={(value) => patchEffects("bass", { bassDb: value })}
				onCommitted={finishSlider}
			/>
			<EffectSlider
				feature="mid"
				selectionCount={segments.length}
				label="Mid"
				value={segment.effects.midDb}
				min={effectLimit("midDb", props.limits)[0]}
				max={effectLimit("midDb", props.limits)[1]}
				step={0.5}
				format={(value) => `${value > 0 ? "+" : ""}${value.toFixed(1)} dB`}
				onChange={(value) => patchEffects("mid", { midDb: value })}
				onCommitted={finishSlider}
			/>
			<EffectSlider
				feature="treble"
				selectionCount={segments.length}
				label="Treble"
				value={segment.effects.trebleDb}
				min={effectLimit("trebleDb", props.limits)[0]}
				max={effectLimit("trebleDb", props.limits)[1]}
				step={0.5}
				format={(value) => `${value > 0 ? "+" : ""}${value.toFixed(1)} dB`}
				onChange={(value) => patchEffects("treble", { trebleDb: value })}
				onCommitted={finishSlider}
			/>

			<EffectGroup
				title="Distortion"
				active={segment.effects.distortionWet > 0}
			>
				<EffectSwitch
					feature="distortion"
					selectionCount={segments.length}
					checked={segment.effects.distortionWet > 0}
					label="Enabled"
					onChange={(enabled) =>
						commitEffects("distortion", {
							distortionWet: enabled
								? Math.max(0.5, segment.effects.distortionWet)
								: 0,
						})
					}
				/>
				<EffectSlider
					feature="distortion"
					selectionCount={segments.length}
					label="Amount"
					value={segment.effects.distortionAmount}
					min={0}
					max={1}
					step={0.01}
					format={formatPercent}
					onChange={(value) =>
						patchEffects("distortion", { distortionAmount: value })
					}
					onCommitted={finishSlider}
					disabled={segment.effects.distortionWet === 0}
				/>
				<EffectSlider
					feature="distortion"
					selectionCount={segments.length}
					label="Wet"
					value={segment.effects.distortionWet}
					min={0}
					max={1}
					step={0.01}
					format={formatPercent}
					onChange={(value) =>
						patchEffects("distortion", { distortionWet: value })
					}
					onCommitted={finishSlider}
					disabled={segment.effects.distortionWet === 0}
				/>
			</EffectGroup>

			<EffectGroup title="Feedback delay" active={segment.effects.delayWet > 0}>
				<EffectSwitch
					feature="delay"
					selectionCount={segments.length}
					checked={segment.effects.delayWet > 0}
					label="Enabled"
					onChange={(enabled) =>
						commitEffects("delay", {
							delayWet: enabled ? Math.max(0.5, segment.effects.delayWet) : 0,
						})
					}
				/>
				<EffectSlider
					feature="delay"
					selectionCount={segments.length}
					label="Time"
					value={segment.effects.delaySeconds}
					min={0}
					max={5}
					step={0.01}
					format={formatSeconds}
					onChange={(value) => patchEffects("delay", { delaySeconds: value })}
					onCommitted={finishSlider}
					disabled={segment.effects.delayWet === 0}
				/>
				<EffectSlider
					feature="delay"
					selectionCount={segments.length}
					label="Feedback"
					value={segment.effects.delayFeedback}
					min={0}
					max={1}
					step={0.01}
					format={formatPercent}
					onChange={(value) => patchEffects("delay", { delayFeedback: value })}
					onCommitted={finishSlider}
					disabled={segment.effects.delayWet === 0}
				/>
				<EffectSlider
					feature="delay"
					selectionCount={segments.length}
					label="Wet"
					value={segment.effects.delayWet}
					min={0}
					max={1}
					step={0.01}
					format={formatPercent}
					onChange={(value) => patchEffects("delay", { delayWet: value })}
					onCommitted={finishSlider}
					disabled={segment.effects.delayWet === 0}
				/>
			</EffectGroup>

			<EffectGroup
				title="Compressor"
				active={segment.effects.compressorEnabled}
			>
				<EffectSwitch
					feature="compressor"
					selectionCount={segments.length}
					checked={segment.effects.compressorEnabled}
					label="Enabled"
					onChange={(compressorEnabled) =>
						commitEffects("compressor", { compressorEnabled })
					}
				/>
				<EffectSlider
					feature="compressor"
					selectionCount={segments.length}
					label="Threshold"
					value={segment.effects.compressorThresholdDb}
					min={-100}
					max={0}
					step={1}
					format={formatDb}
					onChange={(value) =>
						patchEffects("compressor", { compressorThresholdDb: value })
					}
					onCommitted={finishSlider}
					disabled={!segment.effects.compressorEnabled}
				/>
				<EffectSlider
					feature="compressor"
					selectionCount={segments.length}
					label="Knee"
					value={segment.effects.compressorKneeDb}
					min={0}
					max={40}
					step={1}
					format={formatDb}
					onChange={(value) =>
						patchEffects("compressor", { compressorKneeDb: value })
					}
					onCommitted={finishSlider}
					disabled={!segment.effects.compressorEnabled}
				/>
				<EffectSlider
					feature="compressor"
					selectionCount={segments.length}
					label="Ratio"
					value={segment.effects.compressorRatio}
					min={1}
					max={20}
					step={0.5}
					format={(value) => `${value.toFixed(1)}:1`}
					onChange={(value) =>
						patchEffects("compressor", { compressorRatio: value })
					}
					onCommitted={finishSlider}
					disabled={!segment.effects.compressorEnabled}
				/>
				<EffectSlider
					feature="compressor"
					selectionCount={segments.length}
					label="Attack"
					value={segment.effects.compressorAttackSeconds}
					min={0}
					max={1}
					step={0.001}
					format={formatMilliseconds}
					onChange={(value) =>
						patchEffects("compressor", { compressorAttackSeconds: value })
					}
					onCommitted={finishSlider}
					disabled={!segment.effects.compressorEnabled}
				/>
				<EffectSlider
					feature="compressor"
					selectionCount={segments.length}
					label="Release"
					value={segment.effects.compressorReleaseSeconds}
					min={0}
					max={1}
					step={0.01}
					format={formatMilliseconds}
					onChange={(value) =>
						patchEffects("compressor", { compressorReleaseSeconds: value })
					}
					onCommitted={finishSlider}
					disabled={!segment.effects.compressorEnabled}
				/>
			</EffectGroup>

			<EffectGroup title="Chorus" active={segment.effects.chorusEnabled}>
				<EffectSwitch
					feature="chorus"
					selectionCount={segments.length}
					checked={segment.effects.chorusEnabled}
					label="Enabled"
					onChange={(chorusEnabled) =>
						commitEffects("chorus", { chorusEnabled })
					}
				/>
				<EffectSlider
					feature="chorus"
					selectionCount={segments.length}
					label="Frequency"
					value={segment.effects.chorusFrequencyHz}
					min={0}
					max={20}
					step={0.1}
					format={(value) => `${value.toFixed(1)} Hz`}
					onChange={(value) =>
						patchEffects("chorus", { chorusFrequencyHz: value })
					}
					onCommitted={finishSlider}
					disabled={!segment.effects.chorusEnabled}
				/>
				<EffectSlider
					feature="chorus"
					selectionCount={segments.length}
					label="Delay"
					value={segment.effects.chorusDelayMs}
					min={0}
					max={100}
					step={0.5}
					format={(value) => `${value.toFixed(1)} ms`}
					onChange={(value) => patchEffects("chorus", { chorusDelayMs: value })}
					onCommitted={finishSlider}
					disabled={!segment.effects.chorusEnabled}
				/>
				<EffectSlider
					feature="chorus"
					selectionCount={segments.length}
					label="Depth"
					value={segment.effects.chorusDepth}
					min={0}
					max={1}
					step={0.01}
					format={formatPercent}
					onChange={(value) => patchEffects("chorus", { chorusDepth: value })}
					onCommitted={finishSlider}
					disabled={!segment.effects.chorusEnabled}
				/>
				<EffectSlider
					feature="chorus"
					selectionCount={segments.length}
					label="Stereo spread"
					value={segment.effects.chorusSpreadDegrees}
					min={0}
					max={360}
					step={5}
					format={(value) => `${value.toFixed(0)}°`}
					onChange={(value) =>
						patchEffects("chorus", { chorusSpreadDegrees: value })
					}
					onCommitted={finishSlider}
					disabled={!segment.effects.chorusEnabled}
				/>
				<EffectSlider
					feature="chorus"
					selectionCount={segments.length}
					label="Feedback"
					value={segment.effects.chorusFeedback}
					min={0}
					max={1}
					step={0.01}
					format={formatPercent}
					onChange={(value) =>
						patchEffects("chorus", { chorusFeedback: value })
					}
					onCommitted={finishSlider}
					disabled={!segment.effects.chorusEnabled}
				/>
				<EffectSlider
					feature="chorus"
					selectionCount={segments.length}
					label="Wet"
					value={segment.effects.chorusWet}
					min={0}
					max={1}
					step={0.01}
					format={formatPercent}
					onChange={(value) => patchEffects("chorus", { chorusWet: value })}
					onCommitted={finishSlider}
					disabled={!segment.effects.chorusEnabled}
				/>
			</EffectGroup>

			<EffectGroup title="Reverb" active={segment.effects.reverbEnabled}>
				<EffectSwitch
					feature="reverb"
					selectionCount={segments.length}
					checked={segment.effects.reverbEnabled}
					label="Enabled"
					onChange={(reverbEnabled) =>
						commitEffects("reverb", { reverbEnabled })
					}
				/>
				<EffectSlider
					feature="reverb"
					selectionCount={segments.length}
					label="Decay"
					value={segment.effects.reverbDecaySeconds}
					min={0.001}
					max={30}
					step={0.01}
					format={formatSeconds}
					onChange={(value) =>
						patchEffects("reverb", { reverbDecaySeconds: value })
					}
					onCommitted={finishSlider}
					disabled={!segment.effects.reverbEnabled}
				/>
				<EffectSlider
					feature="reverb"
					selectionCount={segments.length}
					label="Pre-delay"
					value={segment.effects.reverbPreDelaySeconds}
					min={0}
					max={5}
					step={0.01}
					format={formatSeconds}
					onChange={(value) =>
						patchEffects("reverb", { reverbPreDelaySeconds: value })
					}
					onCommitted={finishSlider}
					disabled={!segment.effects.reverbEnabled}
				/>
				<EffectSlider
					feature="reverb"
					selectionCount={segments.length}
					label="Wet"
					value={segment.effects.reverbWet}
					min={0}
					max={1}
					step={0.01}
					format={formatPercent}
					onChange={(value) => patchEffects("reverb", { reverbWet: value })}
					onCommitted={finishSlider}
					disabled={!segment.effects.reverbEnabled}
				/>
				<EffectNumberField
					feature="reverb"
					selectionCount={segments.length}
					label="IR seed"
					value={segment.effects.reverbSeed}
					min={0}
					max={0xffff_ffff}
					onChange={(value) => patchEffects("reverb", { reverbSeed: value })}
					onCommitted={finishSlider}
					disabled={!segment.effects.reverbEnabled}
				/>
			</EffectGroup>

			<EffectGroup title="Effect tail" active={segment.effects.tailSeconds > 0}>
				<Typography variant="caption" color="text.secondary" display="block">
					Silence processed after the source ends so delay, reverb, and feedback
					can ring out in playback and exports.
				</Typography>
				<EffectSlider
					feature="tail"
					selectionCount={segments.length}
					label="Duration"
					value={segment.effects.tailSeconds}
					min={0}
					max={30}
					step={0.1}
					format={formatSeconds}
					onChange={(value) =>
						patchResizingEffect("tail", { tailSeconds: value })
					}
					onCommitted={finishSlider}
				/>
			</EffectGroup>

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
					feature="merge"
					selectionCount={segments.length}
					disabled={alreadyMerged}
					disabledReason={
						alreadyMerged ? "This unit is already merged." : undefined
					}
					size="small"
					variant="outlined"
					startIcon={<MergeIcon />}
					onClick={editor.mergeSelected}
				>
					Merge (M)
				</InspectorActionButton>
				{mergedUnit && (
					<InspectorActionButton
						feature="unmerge"
						selectionCount={segments.length}
						size="small"
						variant="outlined"
						startIcon={<CallSplitIcon />}
						onClick={editor.unmergeSelected}
					>
						Ungroup
					</InspectorActionButton>
				)}
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

function EffectGroup(props: {
	title: string;
	active: boolean;
	children: ReactNode;
}) {
	return (
		<Accordion
			variant="outlined"
			disableGutters
			defaultExpanded={props.active}
			sx={{ mt: 1, "&:before": { display: "none" } }}
		>
			<AccordionSummary expandIcon={<ExpandMoreIcon />}>
				<Stack
					direction="row"
					alignItems="baseline"
					justifyContent="space-between"
					sx={{ width: "100%", pr: 1 }}
				>
					<Typography variant="body2">{props.title}</Typography>
					<Typography
						variant="caption"
						color={props.active ? "secondary.main" : "text.disabled"}
					>
						{props.active ? "On" : "Off"}
					</Typography>
				</Stack>
			</AccordionSummary>
			<AccordionDetails sx={{ pt: 0 }}>{props.children}</AccordionDetails>
		</Accordion>
	);
}

function EffectSwitch(props: {
	feature: InspectorFeatureId;
	selectionCount: number;
	checked: boolean;
	label: string;
	onChange: (checked: boolean) => void;
}) {
	const gated = isInspectorFeatureDisabled(props.feature, props.selectionCount);
	return (
		<Tooltip title={gated ? MULTI_SELECTION_DISABLED_REASON : ""}>
			<FormControlLabel
				control={
					<Switch
						size="small"
						checked={props.checked}
						disabled={gated}
						onChange={(_event, checked) => props.onChange(checked)}
					/>
				}
				label={<Typography variant="caption">{props.label}</Typography>}
			/>
		</Tooltip>
	);
}

function EffectNumberField(props: {
	feature: InspectorFeatureId;
	selectionCount: number;
	label: string;
	value: number;
	min: number;
	max: number;
	onChange: (value: number) => void;
	onCommitted: () => void;
	disabled?: boolean;
}) {
	const gated = isInspectorFeatureDisabled(props.feature, props.selectionCount);
	const disabled = props.disabled || gated;
	return (
		<Tooltip title={gated ? MULTI_SELECTION_DISABLED_REASON : ""}>
			<TextField
				size="small"
				fullWidth
				type="number"
				label={props.label}
				value={props.value}
				disabled={disabled}
				onChange={(event) => {
					const parsed = Number(event.target.value);
					if (!Number.isFinite(parsed)) return;
					props.onChange(
						Math.min(props.max, Math.max(props.min, Math.round(parsed))),
					);
				}}
				onBlur={props.onCommitted}
				slotProps={{
					htmlInput: { min: props.min, max: props.max, step: 1 },
				}}
				sx={{ mt: 1 }}
			/>
		</Tooltip>
	);
}

function formatPercent(value: number): string {
	return `${Math.round(value * 100)}%`;
}

function formatDb(value: number): string {
	return `${value.toFixed(1)} dB`;
}

function formatSeconds(value: number): string {
	return `${value.toFixed(value < 0.1 ? 3 : 2)} s`;
}

function formatMilliseconds(value: number): string {
	return `${Math.round(value * 1_000)} ms`;
}

function InspectorActionButton(
	props: ButtonProps & {
		feature: InspectorFeatureId;
		selectionCount: number;
		disabledReason?: string;
	},
) {
	const { feature, selectionCount, disabled, disabledReason, ...buttonProps } =
		props;
	const gated = isInspectorFeatureDisabled(feature, selectionCount);

	return (
		<Tooltip
			title={gated ? MULTI_SELECTION_DISABLED_REASON : (disabledReason ?? "")}
		>
			<span>
				<InspectorButton {...buttonProps} disabled={disabled || gated} />
			</span>
		</Tooltip>
	);
}
