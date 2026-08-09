import type { SegmentEffects } from "./model";

export const SHARED_DSP_EFFECT_CONFIG_VERSION = 2 as const;

/** The only JavaScript-to-WASM effect schema. Rust rejects unknown versions
 * and incomplete objects instead of interpreting them positionally.
 */
export function sharedDspEffectConfig(effects: SegmentEffects) {
	return {
		version: SHARED_DSP_EFFECT_CONFIG_VERSION,
		effects: { ...effects },
	};
}

/** Keep only the length-changing, random-access prefix of the fixed chain. */
export function sharedDspPreprocessEffectConfig(
	effects: SegmentEffects,
): SegmentEffects {
	return {
		...effects,
		volumeDb: 0,
		bassDb: 0,
		midDb: 0,
		trebleDb: 0,
		distortionWet: 0,
		delayWet: 0,
		compressorEnabled: false,
		chorusEnabled: false,
		reverbEnabled: false,
	};
}

/** Keep only the streaming suffix after reverse/pitch/rate/tail. */
export function sharedDspStreamingEffectConfig(
	effects: SegmentEffects,
): SegmentEffects {
	return {
		...effects,
		pitchCents: 0,
		rate: 1,
		tailSeconds: 0,
		reverse: false,
	};
}
