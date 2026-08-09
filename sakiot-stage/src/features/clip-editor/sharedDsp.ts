import initDsp, {
	WasmSegmentProcessor,
} from "../../../../sakiot-DSP/pkg/sakiot_dsp.js";
import type { SegmentEffects, TimelineSegment } from "./model";

const OUTPUT_CHANNELS = 2;
export const SHARED_DSP_EFFECT_CONFIG_VERSION = 2 as const;

let ready = false;
let failed = false;
const initialization = initDsp()
	.then(() => {
		ready = true;
	})
	.catch(() => {
		failed = true;
	});

export interface SharedDspPcm {
	channels: number;
	sampleRate: number;
	frames: number;
	interleaved: Float32Array;
}

interface CachedRender {
	pcm: SharedDspPcm;
	audioBuffer: AudioBuffer | null;
}

const cache = new WeakMap<AudioBuffer, Map<string, CachedRender>>();

/** Begin loading the shared module without delaying initial editor rendering. */
export function warmSharedDsp(): Promise<void> {
	return initialization;
}

export function sharedDspAvailable(): boolean {
	return ready && !failed;
}

/** True after initialization either succeeds or irrecoverably fails. */
export function sharedDspSettled(): boolean {
	return ready || failed;
}

/**
 * The only JavaScript-to-WASM effect schema. Unknown versions and incomplete
 * objects are rejected by Rust instead of being interpreted positionally.
 */
export function sharedDspEffectConfig(effects: SegmentEffects) {
	return {
		version: SHARED_DSP_EFFECT_CONFIG_VERSION,
		effects: { ...effects },
	};
}

/**
 * Render a segment to canonical interleaved PCM. Playback and effect-aware
 * waveform generation share this exact cached result.
 */
export function renderSharedSegmentPcm(
	source: AudioBuffer,
	segment: TimelineSegment,
): SharedDspPcm | null {
	if (!sharedDspAvailable() || typeof source.getChannelData !== "function") {
		return null;
	}
	return renderEntry(source, segment)?.pcm ?? null;
}

/**
 * Render a segment's complete trimmed source through the canonical WASM path.
 * Returns null when WASM initialization failed or the supplied browser audio
 * objects do not provide the APIs needed by the renderer.
 */
export function renderSharedSegment(
	context: AudioContext,
	source: AudioBuffer,
	segment: TimelineSegment,
): AudioBuffer | null {
	if (
		!sharedDspAvailable() ||
		typeof source.getChannelData !== "function" ||
		typeof context.createBuffer !== "function"
	) {
		return null;
	}
	const entry = renderEntry(source, segment);
	if (!entry) return null;
	if (entry.audioBuffer) return entry.audioBuffer;

	const output = context.createBuffer(
		entry.pcm.channels,
		entry.pcm.frames,
		entry.pcm.sampleRate,
	);
	for (let channel = 0; channel < entry.pcm.channels; channel += 1) {
		const channelData = output.getChannelData(channel);
		for (let frame = 0; frame < entry.pcm.frames; frame += 1) {
			channelData[frame] =
				entry.pcm.interleaved[frame * entry.pcm.channels + channel] ?? 0;
		}
	}
	entry.audioBuffer = output;
	return output;
}

function renderEntry(
	source: AudioBuffer,
	segment: TimelineSegment,
): CachedRender | null {
	const key = sharedDspRenderKey(segment);
	const sourceCache = cache.get(source) ?? new Map<string, CachedRender>();
	cache.set(source, sourceCache);
	const cached = sourceCache.get(key);
	if (cached) return cached;

	const startFrame = Math.max(
		0,
		Math.min(source.length, Math.round(segment.sourceIn * source.sampleRate)),
	);
	const endFrame = Math.max(
		startFrame,
		Math.min(source.length, Math.round(segment.sourceOut * source.sampleRate)),
	);
	const frameCount = endFrame - startFrame;
	if (frameCount === 0) return null;
	const interleaved = new Float32Array(frameCount * OUTPUT_CHANNELS);
	const left = source.getChannelData(0);
	const right = source.getChannelData(Math.min(1, source.numberOfChannels - 1));
	for (let frame = 0; frame < frameCount; frame += 1) {
		interleaved[frame * OUTPUT_CHANNELS] = left[startFrame + frame] ?? 0;
		interleaved[frame * OUTPUT_CHANNELS + 1] = right[startFrame + frame] ?? 0;
	}

	const processor = new WasmSegmentProcessor(
		source.sampleRate,
		OUTPUT_CHANNELS,
	);
	try {
		if (!processor.set_effect_config(sharedDspEffectConfig(segment.effects))) {
			return null;
		}
		const rendered = processor.render_clip_interleaved(interleaved);
		if (rendered.length === 0) return null;
		const pcm: SharedDspPcm = {
			channels: OUTPUT_CHANNELS,
			sampleRate: source.sampleRate,
			frames: Math.floor(rendered.length / OUTPUT_CHANNELS),
			interleaved: rendered,
		};
		const entry: CachedRender = { pcm, audioBuffer: null };
		sourceCache.set(key, entry);
		return entry;
	} finally {
		processor.free();
	}
}

export function sharedDspRenderKey(segment: TimelineSegment): string {
	const effect = segment.effects;
	return [
		SHARED_DSP_EFFECT_CONFIG_VERSION,
		segment.sourceIn,
		segment.sourceOut,
		effect.volumeDb,
		effect.pitchCents,
		effect.rate,
		effect.tailSeconds,
		effect.bassDb,
		effect.midDb,
		effect.trebleDb,
		effect.distortionAmount,
		effect.distortionWet,
		effect.delaySeconds,
		effect.delayFeedback,
		effect.delayWet,
		effect.compressorEnabled ? 1 : 0,
		effect.compressorThresholdDb,
		effect.compressorKneeDb,
		effect.compressorRatio,
		effect.compressorAttackSeconds,
		effect.compressorReleaseSeconds,
		effect.chorusEnabled ? 1 : 0,
		effect.chorusFrequencyHz,
		effect.chorusDelayMs,
		effect.chorusDepth,
		effect.chorusSpreadDegrees,
		effect.chorusFeedback,
		effect.chorusWet,
		effect.reverbEnabled ? 1 : 0,
		effect.reverbDecaySeconds,
		effect.reverbPreDelaySeconds,
		effect.reverbWet,
		effect.reverbSeed,
		effect.reverse ? 1 : 0,
	].join(":");
}
