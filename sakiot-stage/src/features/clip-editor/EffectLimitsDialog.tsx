import { useEffect, useState } from "react";
import { BaseDialog } from "../../shared/BaseDialog";
import { Button, TextField } from "../../shared/ui";
import {
	DEFAULT_EFFECT_LIMITS,
	EFFECT_LIMIT_KEYS,
	EFFECT_LIMIT_SAFETY_CAPS,
	type EffectLimits,
} from "./effectLimits";

type LimitsKey = keyof EffectLimits;

const PARAM_LABELS: Record<
	LimitsKey,
	{ label: string; unit: string; step: number }
> = {
	volumeDb: { label: "Volume", unit: "dB", step: 0.5 },
	pitchCents: { label: "Pitch", unit: "ct", step: 10 },
	rate: { label: "Speed", unit: "×", step: 0.05 },
	bassDb: { label: "Bass", unit: "dB", step: 0.5 },
	midDb: { label: "Mid", unit: "dB", step: 0.5 },
	trebleDb: { label: "Treble", unit: "dB", step: 0.5 },
};

const clamp = (value: number, range: readonly [number, number]): number =>
	Math.min(range[1], Math.max(range[0], value));

function PairFieldRow(props: {
	param: LimitsKey;
	limits: EffectLimits;
	error: boolean;
	onDraft: (param: LimitsKey, min: number, max: number) => void;
}) {
	const { label, unit, step } = PARAM_LABELS[props.param];
	const [minimum, maximum] = props.limits[props.param];
	const caps = EFFECT_LIMIT_SAFETY_CAPS[props.param];
	return (
		<div className="mb-3">
			<span className="text-muted text-xs leading-5">
				{label} ({unit})
			</span>
			<div className="flex flex-row gap-2 mt-1">
				<TextField
					label="Min"
					type="number"
					value={String(minimum)}
					isInvalid={props.error}
					onChange={(value) => {
						const parsed = Number(value);
						if (!Number.isFinite(parsed)) return;
						props.onDraft(props.param, clamp(parsed, caps), maximum);
					}}
					min={caps[0]}
					max={caps[1]}
					step={step}
				/>
				<TextField
					label="Max"
					type="number"
					value={String(maximum)}
					isInvalid={props.error}
					onChange={(value) => {
						const parsed = Number(value);
						if (!Number.isFinite(parsed)) return;
						props.onDraft(props.param, minimum, clamp(parsed, caps));
					}}
					min={caps[0]}
					max={caps[1]}
					step={step}
				/>
			</div>
			{props.error && (
				<span className="text-danger text-xs leading-5">
					Min must be below max.
				</span>
			)}
		</div>
	);
}

/**
 * Adjusts the slider bounds for Volume, Pitch, Speed, and the Bass/Mid/Treble
 * EQ. Changes apply immediately to the inspector and are saved per user; the
 * server validates exports against the same limits, hard-capped so renders
 * cannot overflow or exhaust memory.
 */
export function EffectLimitsDialog(props: {
	open: boolean;
	onClose: () => void;
	limits: EffectLimits;
	onChange: (limits: EffectLimits) => void;
}) {
	const [draft, setDraft] = useState<EffectLimits>(props.limits);

	useEffect(() => {
		if (props.open) setDraft(props.limits);
	}, [props.open, props.limits]);

	const applyPair = (param: LimitsKey, min: number, max: number) => {
		const next: EffectLimits = {
			...draft,
			[param]: [min, max],
		};
		setDraft(next);
		// Hold back pairs whose min is no longer below max; the field keeps
		// the typed value so the user can fix the partner instead of having
		// it silently reverted.
		if (min < max) props.onChange(next);
	};

	const reset = () => {
		const defaults = structuredClone(DEFAULT_EFFECT_LIMITS);
		setDraft(defaults);
		props.onChange(defaults);
	};

	const invalidPairs = EFFECT_LIMIT_KEYS.filter((param) => {
		const [minimum, maximum] = draft[param];
		return minimum >= maximum;
	});

	return (
		<BaseDialog
			open={props.open}
			onClose={props.onClose}
			title="Effect limits"
			actions={
				<>
					<Button onPress={reset}>Reset to defaults</Button>
					<Button variant="primary" onPress={props.onClose}>
						Done
					</Button>
				</>
			}
		>
			<p className="text-muted text-sm mb-3">
				Set the slider bounds for these effects. Changes apply immediately and
				are saved for this browser. Hard limits: volume and EQ ±240 dB, pitch
				±4800 ct, speed 0.1–10×.
			</p>
			{EFFECT_LIMIT_KEYS.map((param) => (
				<PairFieldRow
					key={param}
					param={param}
					limits={draft}
					error={invalidPairs.includes(param)}
					onDraft={applyPair}
				/>
			))}
			{invalidPairs.length > 0 && (
				<span className="text-muted text-xs leading-5">
					Fix the highlighted pairs to apply them.
				</span>
			)}
		</BaseDialog>
	);
}
