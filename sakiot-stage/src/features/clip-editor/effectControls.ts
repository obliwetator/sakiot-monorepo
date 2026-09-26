import type { EffectLimits } from "./effectLimits";
import type { InspectorFeatureId } from "./inspectorFeaturePolicy";
import type { SegmentEffects } from "./model";

type NumericEffectKey = {
	[K in keyof SegmentEffects]: SegmentEffects[K] extends number ? K : never;
}[keyof SegmentEffects];

type Format = (value: number) => string;

const percent: Format = (value) => `${Math.round(value * 100)}%`;
const db: Format = (value) => `${value.toFixed(1)} dB`;
/** EQ bands read as offsets, so the sign is worth showing even when positive. */
const signedDb: Format = (value) =>
	`${value > 0 ? "+" : ""}${value.toFixed(1)} dB`;
const seconds: Format = (value) => `${value.toFixed(value < 0.1 ? 3 : 2)} s`;
const milliseconds: Format = (value) => `${Math.round(value * 1_000)} ms`;
const hz: Format = (value) => `${value.toFixed(1)} Hz`;
const ms: Format = (value) => `${value.toFixed(1)} ms`;
const degrees: Format = (value) => `${value.toFixed(0)}°`;
const ratio: Format = (value) => `${value.toFixed(1)}:1`;
const cents: Format = (value) =>
	value === 0 ? "0" : `${value > 0 ? "+" : ""}${value} ct`;
const times: Format = (value) => `${value.toFixed(2)}×`;

/** A fixed pair, or the user-adjustable bounds kept under an effect limit. */
type ControlRange = readonly [number, number] | { limit: keyof EffectLimits };

interface SliderControl {
	kind: "slider";
	label: string;
	key: NumericEffectKey;
	range: ControlRange;
	step: number;
	format: Format;
	/**
	 * Speed and tail change how much timeline a segment occupies, so they are
	 * applied through the resizing patch instead of a plain effect patch.
	 */
	resizes?: boolean;
}

interface NumberControl {
	kind: "number";
	label: string;
	key: NumericEffectKey;
	range: readonly [number, number];
}

export type EffectControl = SliderControl | NumberControl;

const slider = (
	label: string,
	key: NumericEffectKey,
	range: ControlRange,
	step: number,
	format: Format,
	resizes = false,
): SliderControl => ({
	kind: "slider",
	label,
	key,
	range,
	step,
	format,
	resizes,
});

const limit = (key: keyof EffectLimits) => ({ limit: key });

/** The always-visible controls above the collapsible effect groups. */
export const BASE_EFFECT_CONTROLS: (SliderControl & {
	feature: InspectorFeatureId;
})[] = [
	{
		...slider("Volume", "volumeDb", limit("volumeDb"), 0.5, db),
		feature: "volume",
	},
	{
		...slider("Pitch", "pitchCents", limit("pitchCents"), 10, cents),
		feature: "pitch",
	},
	{
		...slider("Speed", "rate", limit("rate"), 0.05, times, true),
		feature: "speed",
	},
	{
		...slider("Bass", "bassDb", limit("bassDb"), 0.5, signedDb),
		feature: "bass",
	},
	{ ...slider("Mid", "midDb", limit("midDb"), 0.5, signedDb), feature: "mid" },
	{
		...slider("Treble", "trebleDb", limit("trebleDb"), 0.5, signedDb),
		feature: "treble",
	},
];

export interface EffectGroupSpec {
	title: string;
	feature: InspectorFeatureId;
	/** The field that decides whether the group reads as On. */
	activeKey: keyof SegmentEffects;
	/**
	 * The Enabled switch. A numeric group toggles its wet level, rising to
	 * `enableTo` only when the user's own value is lower; a flag group writes
	 * the boolean directly. Groups without a switch are driven by their own
	 * controls and never disable them.
	 */
	toggle?: { enableTo: number } | { flag: true };
	note?: string;
	controls: EffectControl[];
}

export const EFFECT_GROUPS: EffectGroupSpec[] = [
	{
		title: "Distortion",
		feature: "distortion",
		activeKey: "distortionWet",
		toggle: { enableTo: 0.5 },
		controls: [
			slider("Amount", "distortionAmount", [0, 1], 0.01, percent),
			slider("Wet", "distortionWet", [0, 1], 0.01, percent),
		],
	},
	{
		title: "Feedback delay",
		feature: "delay",
		activeKey: "delayWet",
		toggle: { enableTo: 0.5 },
		controls: [
			slider("Time", "delaySeconds", [0, 5], 0.01, seconds),
			slider("Feedback", "delayFeedback", [0, 1], 0.01, percent),
			slider("Wet", "delayWet", [0, 1], 0.01, percent),
		],
	},
	{
		title: "Compressor",
		feature: "compressor",
		activeKey: "compressorEnabled",
		toggle: { flag: true },
		controls: [
			slider("Threshold", "compressorThresholdDb", [-100, 0], 1, db),
			slider("Knee", "compressorKneeDb", [0, 40], 1, db),
			slider("Ratio", "compressorRatio", [1, 20], 0.5, ratio),
			slider("Attack", "compressorAttackSeconds", [0, 1], 0.001, milliseconds),
			slider("Release", "compressorReleaseSeconds", [0, 1], 0.01, milliseconds),
		],
	},
	{
		title: "Chorus",
		feature: "chorus",
		activeKey: "chorusEnabled",
		toggle: { flag: true },
		controls: [
			slider("Frequency", "chorusFrequencyHz", [0, 20], 0.1, hz),
			slider("Delay", "chorusDelayMs", [0, 100], 0.5, ms),
			slider("Depth", "chorusDepth", [0, 1], 0.01, percent),
			slider("Stereo spread", "chorusSpreadDegrees", [0, 360], 5, degrees),
			slider("Feedback", "chorusFeedback", [0, 1], 0.01, percent),
			slider("Wet", "chorusWet", [0, 1], 0.01, percent),
		],
	},
	{
		title: "Reverb",
		feature: "reverb",
		activeKey: "reverbEnabled",
		toggle: { flag: true },
		controls: [
			slider("Decay", "reverbDecaySeconds", [0.001, 30], 0.01, seconds),
			slider("Pre-delay", "reverbPreDelaySeconds", [0, 5], 0.01, seconds),
			slider("Wet", "reverbWet", [0, 1], 0.01, percent),
			{
				kind: "number",
				label: "IR seed",
				key: "reverbSeed",
				range: [0, 0xffff_ffff],
			},
		],
	},
	{
		title: "Effect tail",
		feature: "tail",
		activeKey: "tailSeconds",
		note: "Silence processed after the source ends so delay, reverb, and feedback can ring out in playback and exports.",
		controls: [slider("Duration", "tailSeconds", [0, 30], 0.1, seconds, true)],
	},
];
