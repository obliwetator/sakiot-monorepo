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
import {
	BASE_EFFECT_CONTROLS,
	EFFECT_GROUPS,
	type EffectControl,
	type EffectGroupSpec,
} from "./effectControls";
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
				"w-full min-[900px]:w-65 min-[900px]:[max-height:none] min-[900px]:flex-none min-h-0 min-w-0 overflow-y-auto overflow-x-hidden border-l-0 min-[900px]:border-l border-t min-[900px]:border-t-0 border-ui-border p-2 min-[900px]:p-4",
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

	/** Flip a group on or off, raising a wet level only when it sits lower. */
	const toggleGroup = (group: EffectGroupSpec, enabled: boolean) => {
		const toggle = group.toggle;
		if (!toggle) return;
		if ("flag" in toggle) {
			commitEffects(group.feature, {
				[group.activeKey]: enabled,
			} as Partial<SegmentEffects>);
			return;
		}
		const current = segment.effects[group.activeKey] as number;
		commitEffects(group.feature, {
			[group.activeKey]: enabled ? Math.max(toggle.enableTo, current) : 0,
		} as Partial<SegmentEffects>);
	};

	const renderControl = (
		control: EffectControl,
		feature: InspectorFeatureId,
		disabled: boolean,
	) => {
		const value = segment.effects[control.key];
		const patch = (next: number) =>
			({ [control.key]: next }) as Partial<SegmentEffects>;
		if (control.kind === "number") {
			return (
				<EffectNumberField
					key={control.key}
					feature={feature}
					selectionCount={segments.length}
					label={control.label}
					value={value}
					min={control.range[0]}
					max={control.range[1]}
					onChange={(next) => patchEffects(feature, patch(next))}
					onCommitted={finishSlider}
					disabled={disabled}
				/>
			);
		}
		const [min, max] =
			"limit" in control.range
				? effectLimit(control.range.limit, props.limits)
				: control.range;
		const apply = control.resizes ? patchResizingEffect : patchEffects;
		return (
			<EffectSlider
				key={control.key}
				feature={feature}
				selectionCount={segments.length}
				label={control.label}
				value={value}
				min={min}
				max={max}
				step={control.step}
				format={control.format}
				onChange={(next) => apply(feature, patch(next))}
				onCommitted={finishSlider}
				disabled={disabled}
			/>
		);
	};

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
			{BASE_EFFECT_CONTROLS.map((control) =>
				renderControl(control, control.feature, false),
			)}

			{EFFECT_GROUPS.map((group) => {
				const on = Boolean(segment.effects[group.activeKey]);
				return (
					<EffectGroup key={group.title} title={group.title} active={on}>
						{group.note && (
							<span className="text-muted block text-xs leading-5">
								{group.note}
							</span>
						)}
						{group.toggle && (
							<EffectSwitch
								feature={group.feature}
								selectionCount={segments.length}
								checked={on}
								label="Enabled"
								onChange={(enabled) => toggleGroup(group, enabled)}
							/>
						)}
						{group.controls.map((control) =>
							renderControl(
								control,
								group.feature,
								Boolean(group.toggle) && !on,
							),
						)}
					</EffectGroup>
				);
			})}

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
						disabled ? "text-slate-500" : "text-muted",
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
							props.active ? "text-creative" : "text-slate-500",
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
