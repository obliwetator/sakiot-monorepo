import { useEffect, useState } from "react";
import { BaseDialog } from "../../shared/BaseDialog";
import {
	Box,
	Button,
	Stack,
	LegacyTextField as TextField,
	Typography,
} from "../../shared/ui";
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
		<Box sx={{ mb: 1.5 }}>
			<Typography variant="caption" color="text.secondary">
				{label} ({unit})
			</Typography>
			<Stack direction="row" spacing={1} sx={{ mt: 0.5 }}>
				<TextField
					size="small"
					label="Min"
					type="number"
					value={minimum}
					error={props.error}
					slotProps={{
						htmlInput: { min: caps[0], max: caps[1], step },
					}}
					onChange={(event) => {
						const parsed = Number(event.currentTarget.value);
						if (!Number.isFinite(parsed)) return;
						props.onDraft(props.param, clamp(parsed, caps), maximum);
					}}
				/>
				<TextField
					size="small"
					label="Max"
					type="number"
					value={maximum}
					error={props.error}
					slotProps={{
						htmlInput: { min: caps[0], max: caps[1], step },
					}}
					onChange={(event) => {
						const parsed = Number(event.currentTarget.value);
						if (!Number.isFinite(parsed)) return;
						props.onDraft(props.param, minimum, clamp(parsed, caps));
					}}
				/>
			</Stack>
			{props.error && (
				<Typography variant="caption" color="error">
					Min must be below max.
				</Typography>
			)}
		</Box>
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
					<Button onClick={reset}>Reset to defaults</Button>
					<Button variant="contained" onClick={props.onClose}>
						Done
					</Button>
				</>
			}
		>
			<Typography variant="body2" color="text.secondary" sx={{ mb: 1.5 }}>
				Set the slider bounds for these effects. Changes apply immediately and
				are saved for this browser. Hard limits: volume and EQ ±240 dB, pitch
				±4800 ct, speed 0.1–10×.
			</Typography>
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
				<Typography variant="caption" color="text.secondary">
					Fix the highlighted pairs to apply them.
				</Typography>
			)}
		</BaseDialog>
	);
}
