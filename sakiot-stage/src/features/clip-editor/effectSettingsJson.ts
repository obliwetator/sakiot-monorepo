import { DEFAULT_EFFECTS, type SegmentEffects } from "./model";

type NumberRange = readonly [minimum: number, maximum: number];

const NUMBER_RANGES = {
	volumeDb: [-40, 12],
	pitchCents: [-1200, 1200],
	rate: [0.5, 2],
	tailSeconds: [0, 30],
	bassDb: [-12, 12],
	midDb: [-12, 12],
	trebleDb: [-12, 12],
	distortionAmount: [0, 1],
	distortionWet: [0, 1],
	delaySeconds: [0, 5],
	delayFeedback: [0, 1],
	delayWet: [0, 1],
	compressorThresholdDb: [-100, 0],
	compressorKneeDb: [0, 40],
	compressorRatio: [1, 20],
	compressorAttackSeconds: [0, 1],
	compressorReleaseSeconds: [0, 1],
	chorusFrequencyHz: [0, 20],
	chorusDelayMs: [0, 100],
	chorusDepth: [0, 1],
	chorusSpreadDegrees: [0, 360],
	chorusFeedback: [0, 1],
	chorusWet: [0, 1],
	reverbDecaySeconds: [0.001, 30],
	reverbPreDelaySeconds: [0, 5],
	reverbWet: [0, 1],
	reverbSeed: [0, 0xffff_ffff],
} as const satisfies Partial<Record<keyof SegmentEffects, NumberRange>>;

export type EffectSettingsJsonResult =
	| { ok: true; patch: Partial<SegmentEffects> }
	| { ok: false; error: string };

function unwrapMarkdownFence(input: string): string {
	const trimmed = input.trim();
	const match = /^```(?:json)?\s*([\s\S]*?)\s*```$/i.exec(trimmed);
	return match?.[1] ?? trimmed;
}

/** Parse and validate a complete or partial camelCase SegmentEffects object. */
export function parseEffectSettingsJson(
	input: string,
): EffectSettingsJsonResult {
	let parsed: unknown;
	try {
		parsed = JSON.parse(unwrapMarkdownFence(input));
	} catch (error) {
		return {
			ok: false,
			error: `Invalid JSON: ${error instanceof Error ? error.message : String(error)}`,
		};
	}
	if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
		return { ok: false, error: "Settings must be a JSON object." };
	}

	const entries = Object.entries(parsed);
	if (entries.length === 0) {
		return { ok: false, error: "Include at least one effect setting." };
	}
	const patch: Record<string, number | boolean> = {};
	for (const [key, value] of entries) {
		if (!Object.hasOwn(DEFAULT_EFFECTS, key)) {
			return { ok: false, error: `Unknown effect setting “${key}”.` };
		}
		const effectKey = key as keyof SegmentEffects;
		const expected = DEFAULT_EFFECTS[effectKey];
		if (typeof value !== typeof expected) {
			return {
				ok: false,
				error: `“${key}” must be a ${typeof expected}.`,
			};
		}
		if (typeof value === "number") {
			if (!Number.isFinite(value)) {
				return { ok: false, error: `“${key}” must be finite.` };
			}
			const range = NUMBER_RANGES[effectKey as keyof typeof NUMBER_RANGES];
			if (range && (value < range[0] || value > range[1])) {
				return {
					ok: false,
					error: `“${key}” must be between ${range[0]} and ${range[1]}.`,
				};
			}
			if (effectKey === "reverbSeed" && !Number.isInteger(value)) {
				return { ok: false, error: "“reverbSeed” must be an integer." };
			}
		}
		patch[key] = value as number | boolean;
	}
	return { ok: true, patch: patch as Partial<SegmentEffects> };
}

export function isEffectSettingsJsonShortcut(
	event: Pick<
		KeyboardEvent,
		"altKey" | "ctrlKey" | "key" | "metaKey" | "shiftKey"
	>,
): boolean {
	return (
		(event.ctrlKey || event.metaKey) &&
		event.shiftKey &&
		!event.altKey &&
		event.key.toLowerCase() === "o"
	);
}
