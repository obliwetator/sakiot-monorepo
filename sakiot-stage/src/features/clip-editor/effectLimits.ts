type NumberRange = readonly [minimum: number, maximum: number];

/**
 * User-adjustable slider bounds for the segment effects that can safely go
 * beyond the original defaults. Persisted per user in localStorage and sent
 * with each compose request so the server validates against the same range.
 */
export type EffectLimits = {
	volumeDb: NumberRange;
	pitchCents: NumberRange;
	rate: NumberRange;
	bassDb: NumberRange;
	midDb: NumberRange;
	trebleDb: NumberRange;
};

export const EFFECT_LIMIT_KEYS = [
	"volumeDb",
	"pitchCents",
	"rate",
	"bassDb",
	"midDb",
	"trebleDb",
] as const satisfies readonly (keyof EffectLimits)[];

/**
 * Absolute bounds the renderers can survive: beyond these the DSP either
 * overflows f32 to INF/NaN (volume/EQ past ~±770 dB) or allocates hundreds of
 * megabytes per segment (pitch beyond 16x resampling). Slider limits are
 * clamped to these and the server rejects any request outside them.
 */
export const EFFECT_LIMIT_SAFETY_CAPS: EffectLimits = {
	volumeDb: [-240, 240],
	pitchCents: [-4800, 4800],
	rate: [0.1, 10],
	bassDb: [-240, 240],
	midDb: [-240, 240],
	trebleDb: [-240, 240],
};

/** Doubled from the original ranges: volume -40..12, pitch ±1200, rate
 * 0.5..2, and ±12 dB EQ. */
export const DEFAULT_EFFECT_LIMITS: EffectLimits = {
	volumeDb: [-80, 24],
	pitchCents: [-2400, 2400],
	rate: [0.25, 4],
	bassDb: [-24, 24],
	midDb: [-24, 24],
	trebleDb: [-24, 24],
};

const STORAGE_KEY = "sakiot:clip-editor:effect-limits";

const clamp = (value: number, range: NumberRange): number =>
	Math.min(range[1], Math.max(range[0], value));

/** Clamp a range to the safety caps, keeping the fallback when invalid. */
export function clampLimitToSafety(
	key: keyof EffectLimits,
	range: NumberRange,
): NumberRange {
	const [minimum, maximum] = range;
	if (
		!Number.isFinite(minimum) ||
		!Number.isFinite(maximum) ||
		minimum >= maximum
	) {
		return DEFAULT_EFFECT_LIMITS[key];
	}
	return [
		clamp(minimum, EFFECT_LIMIT_SAFETY_CAPS[key]),
		clamp(maximum, EFFECT_LIMIT_SAFETY_CAPS[key]),
	];
}

/** The effective bounds for one effect parameter, clamped to safety caps. */
export function effectLimit(
	key: keyof EffectLimits,
	limits: EffectLimits,
): NumberRange {
	return clampLimitToSafety(key, limits[key]);
}

export interface StorageLike {
	getItem(key: string): string | null;
	setItem(key: string, value: string): void;
}

const defaultStorage = (): StorageLike | null =>
	typeof globalThis.localStorage === "undefined"
		? null
		: globalThis.localStorage;

function loadRange(
	key: keyof EffectLimits,
	value: unknown,
	fallback: NumberRange,
): NumberRange {
	if (!Array.isArray(value) || value.length !== 2) return fallback;
	const [minimum, maximum] = value;
	if (
		typeof minimum !== "number" ||
		typeof maximum !== "number" ||
		!Number.isFinite(minimum) ||
		!Number.isFinite(maximum) ||
		minimum >= maximum
	) {
		return fallback;
	}
	return clampLimitToSafety(key, [minimum, maximum]);
}

/** Load the saved limits, or the doubled defaults when unset or corrupted. */
export function loadEffectLimits(
	storage: StorageLike | null = defaultStorage(),
): EffectLimits {
	const limits = structuredClone(DEFAULT_EFFECT_LIMITS);
	if (!storage) return limits;
	const raw = storage.getItem(STORAGE_KEY);
	if (!raw) return limits;
	try {
		const parsed = JSON.parse(raw) as Record<string, unknown>;
		for (const key of EFFECT_LIMIT_KEYS) {
			limits[key] = loadRange(key, parsed[key], limits[key]);
		}
		return limits;
	} catch {
		return limits;
	}
}

/** Persist the adjusted limits; failures (quota, privacy mode) are swallowed. */
export function saveEffectLimits(
	limits: EffectLimits,
	storage: StorageLike | null = defaultStorage(),
): void {
	if (!storage) return;
	try {
		storage.setItem(STORAGE_KEY, JSON.stringify(limits));
	} catch {
		// Storage is best-effort; ignore quota and availability errors.
	}
}

/** The snake_case request shape the compose endpoint validates against. */
export function serializeLimits(limits: EffectLimits) {
	return {
		volume_db_min: limits.volumeDb[0],
		volume_db_max: limits.volumeDb[1],
		pitch_cents_min: limits.pitchCents[0],
		pitch_cents_max: limits.pitchCents[1],
		rate_min: limits.rate[0],
		rate_max: limits.rate[1],
		bass_db_min: limits.bassDb[0],
		bass_db_max: limits.bassDb[1],
		mid_db_min: limits.midDb[0],
		mid_db_max: limits.midDb[1],
		treble_db_min: limits.trebleDb[0],
		treble_db_max: limits.trebleDb[1],
	};
}
