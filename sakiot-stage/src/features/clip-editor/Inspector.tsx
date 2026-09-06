import {
	GitBranch as CallSplitIcon,
	Scissors as ContentCutIcon,
	Trash2 as DeleteIcon,
	ChevronDown as ExpandMoreIcon,
	GitMerge as MergeIcon,
	RotateCcw as ReplayIcon,
	Settings as SettingsIcon,
} from "lucide-react";
import { type ReactNode, useId } from "react";
import {
	type ButtonProps,
	cn,
	Disclosure,
	DisclosurePanel,
	DisclosureTrigger,
	IconButton,
	Button as InspectorButton,
	Slider,
	Switch,
	TextField,
	Tooltip,
	TooltipTrigger,
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
	className: "transition-none",
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
		<aside
			aria-label="Inspector"
			data-testid="clip-inspector"
			className={cn(
				"w-full min-[900px]:w-65 min-[900px]:[max-height:none] min-[900px]:flex-none min-h-0 min-w-0 overflow-y-auto overflow-x-hidden [border-left:0px_solid] min-[900px]:border-l border-t min-[900px]:[border-top:0px_solid] border-ui-border p-2 min-[900px]:p-4",
				segment ? "[max-height:33.333%]" : "[max-height:none]",
				segment ? "[flex:0_0_33.333%]" : "flex-none",
			)}
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
					<p className="leading-6 text-muted">Inspector</p>
					<p className="text-muted text-sm mt-2">No segment selected.</p>
					<span className="text-muted block text-xs leading-5 mt-1">
						Click a clip on the timeline to trim it and adjust its effects.
					</span>
				</>
			)}
		</aside>
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
			<div className="flex items-center justify-between flex-row">
				<p className="leading-6 text-muted">
					{multi ? `${segments.length} segments selected` : "Selected segment"}
				</p>
				<TooltipTrigger delay={400}>
					<IconButton
						aria-label={"Adjust the effect limits (volume, pitch, speed, EQ)"}
						size="sm"
						onPress={props.onOpenLimits}
					>
						<SettingsIcon size={16} />
					</IconButton>
					<Tooltip>
						{"Adjust the effect limits (volume, pitch, speed, EQ)"}
					</Tooltip>
				</TooltipTrigger>
			</div>
			<h6
				title={
					multi
						? segments.map((s) => props.clipName(s.sourceId)).join(", ")
						: props.clipName(segment.sourceId)
				}
				className="font-medium tracking-[0.001em] text-xl truncate"
			>
				{props.clipName(segment.sourceId)}
				{multi ? ` +${segments.length - 1}` : ""}
			</h6>
			<span className="text-muted text-xs leading-5">
				{multi
					? "Effect changes apply to all selected segments."
					: `${formatDuration(duration)} · at ${formatDuration(segment.timelineStart)}`}
			</span>
			{mergedUnit && (
				<span className="text-muted block text-xs leading-5">
					Merged unit: the clips act as one element. Ungroup to edit them
					individually.
				</span>
			)}

			<hr className="w-full border-t border-ui-border my-4" />

			<p className="leading-6 text-muted">Effects</p>

			{multi && (
				<div className="mt-2">
					<p className="text-muted text-sm">
						Editing effects for several segments at once can shift their boxes
						unexpectedly.
					</p>
				</div>
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
				<span className="text-muted block text-xs leading-5">
					Silence processed after the source ends so delay, reverb, and feedback
					can ring out in playback and exports.
				</span>
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

			<hr className="w-full border-t border-ui-border my-4" />

			<div className="flex flex-row flex-wrap gap-1">
				<InspectorActionButton
					feature="split"
					selectionCount={segments.length}
					size="sm"
					variant="outline"
					onPress={editor.splitSelectedAtPlayhead}
				>
					<ContentCutIcon />
					Split (S)
				</InspectorActionButton>
				<InspectorActionButton
					feature="merge"
					selectionCount={segments.length}
					disabledReason={
						alreadyMerged ? "This unit is already merged." : undefined
					}
					size="sm"
					variant="outline"
					onPress={editor.mergeSelected}
					isDisabled={alreadyMerged}
				>
					<MergeIcon />
					Merge (M)
				</InspectorActionButton>
				{mergedUnit && (
					<InspectorActionButton
						feature="unmerge"
						selectionCount={segments.length}
						size="sm"
						variant="outline"
						onPress={editor.unmergeSelected}
					>
						<CallSplitIcon />
						Ungroup
					</InspectorActionButton>
				)}
				<InspectorActionButton
					feature="reverse"
					selectionCount={segments.length}
					size="sm"
					variant={segment.effects.reverse ? "primary" : "outline"}
					aria-pressed={segment.effects.reverse}
					onPress={editor.toggleReverse}
				>
					<ReplayIcon />
					Reverse (R)
				</InspectorActionButton>
				<InspectorActionButton
					feature="delete"
					selectionCount={segments.length}
					size="sm"
					variant="danger"
					onPress={editor.removeSelected}
				>
					<DeleteIcon />
					Delete
				</InspectorActionButton>
			</div>
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
	const helpId = useId();
	const gated = isInspectorFeatureDisabled(props.feature, props.selectionCount);
	const disabled = props.disabled || gated;

	return (
		<div className="min-w-0">
			<div className="mb-2">
				<span
					className={cn(
						"text-xs leading-5",
						disabled ? "[color:#64748b]" : "text-muted",
					)}
				>
					{props.label} · {props.format(props.value)}
				</span>
				<Slider
					aria-describedby={helpId}
					{...SLIDER_PROPS}
					step={props.step}
					value={props.value}
					aria-label={props.label}
					minValue={props.min}
					maxValue={props.max}
					isDisabled={disabled}
					onChangeEnd={props.onCommitted}
					onChange={(value) => props.onChange(Number(value))}
				/>
			</div>
			<p id={helpId} className="mt-1 text-xs text-muted">
				{gated ? MULTI_SELECTION_DISABLED_REASON : ""}
			</p>
		</div>
	);
}

function EffectGroup(props: {
	title: string;
	active: boolean;
	children: ReactNode;
}) {
	return (
		<Disclosure className="mt-2 before:hidden" defaultExpanded={props.active}>
			<DisclosureTrigger icon={<ExpandMoreIcon />}>
				<div className="flex items-baseline justify-between flex-row w-full pr-2">
					<p className="text-sm">{props.title}</p>
					<span
						className={cn(
							"text-xs leading-5",
							props.active ? "text-creative" : "[color:#64748b]",
						)}
					>
						{props.active ? "On" : "Off"}
					</span>
				</div>
			</DisclosureTrigger>
			<DisclosurePanel className="pt-0">{props.children}</DisclosurePanel>
		</Disclosure>
	);
}

function EffectSwitch(props: {
	feature: InspectorFeatureId;
	selectionCount: number;
	checked: boolean;
	label: string;
	onChange: (checked: boolean) => void;
}) {
	const helpId = useId();
	const gated = isInspectorFeatureDisabled(props.feature, props.selectionCount);
	return (
		<div className="min-w-0">
			<Switch
				aria-describedby={helpId}
				isSelected={props.checked}
				isDisabled={gated}
				onChange={(checked) => props.onChange(checked)}
			>
				<span className={"text-xs leading-5"}>{props.label}</span>
			</Switch>
			<p id={helpId} className="mt-1 text-xs text-muted">
				{gated ? MULTI_SELECTION_DISABLED_REASON : ""}
			</p>
		</div>
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
	const helpId = useId();
	const gated = isInspectorFeatureDisabled(props.feature, props.selectionCount);
	const disabled = props.disabled || gated;
	return (
		<div className="min-w-0">
			<TextField
				aria-describedby={helpId}
				type="number"
				label={props.label}
				value={String(props.value)}
				onBlur={props.onCommitted}
				className="mt-2"
				isDisabled={disabled}
				onChange={(value) => {
					const parsed = Number(value);
					if (!Number.isFinite(parsed)) return;
					props.onChange(
						Math.min(props.max, Math.max(props.min, Math.round(parsed))),
					);
				}}
				min={props.min}
				max={props.max}
				step={1}
			/>
			<p id={helpId} className="mt-1 text-xs text-muted">
				{gated ? MULTI_SELECTION_DISABLED_REASON : ""}
			</p>
		</div>
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
	const {
		feature,
		selectionCount,
		isDisabled,
		disabledReason,
		...buttonProps
	} = props;
	const helpId = useId();
	const gated = isInspectorFeatureDisabled(feature, selectionCount);

	return (
		<div className="min-w-0">
			<InspectorButton
				aria-describedby={helpId}
				{...buttonProps}
				isDisabled={isDisabled || gated}
			/>
			<p id={helpId} className="mt-1 text-xs text-muted">
				{gated ? MULTI_SELECTION_DISABLED_REASON : (disabledReason ?? "")}
			</p>
		</div>
	);
}
