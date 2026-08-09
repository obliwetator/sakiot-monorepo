import { describe, expect, test } from "bun:test";
import { WasmSegmentProcessor } from "../../../../sakiot-DSP/pkg/sakiot_dsp.js";
import type { SegmentEffects } from "./model";
import { DEFAULT_EFFECTS, type TimelineSegment } from "./model";
import {
	requestSharedSegmentPreprocessedPcm,
	requestSharedSegmentRender,
	sharedDspEffectConfig,
	warmSharedDsp,
} from "./sharedDsp";
import {
	initializeSharedDspWorkerRuntime,
	preprocessSharedDspWorkerRequest,
	processSharedDspWorkerRequest,
} from "./sharedDspWorkerRuntime";

async function renderThroughSplit(
	id: number,
	sampleRate: number,
	left: Float32Array,
	right: Float32Array,
	effects: SegmentEffects,
) {
	const pcm = await preprocessSharedDspWorkerRequest({
		type: "preprocess",
		id,
		sampleRate,
		left,
		right,
		effects,
	});
	return processSharedDspWorkerRequest({
		type: "process",
		id,
		pcm,
		effects,
	});
}

class TestAudioBuffer {
	readonly channels: Float32Array[];

	constructor(
		readonly numberOfChannels: number,
		readonly length: number,
		readonly sampleRate: number,
	) {
		this.channels = Array.from(
			{ length: numberOfChannels },
			() => new Float32Array(length),
		);
	}

	getChannelData(channel: number): Float32Array {
		return this.channels[channel] ?? new Float32Array();
	}
}

describe("shared browser DSP worker runtime", () => {
	test("transfers a render through the worker and reuses its cache", async () => {
		await warmSharedDsp();
		const source = new TestAudioBuffer(2, 512, 48_000);
		for (let frame = 0; frame < source.length; frame += 1) {
			(source.channels[0] ?? new Float32Array())[frame] = frame / source.length;
			(source.channels[1] ?? new Float32Array())[frame] =
				-frame / source.length;
		}
		const segment: TimelineSegment = {
			id: "worker-cache-segment",
			track: 0,
			source: "clip",
			sourceId: "worker-cache-source",
			sourceIn: 64 / 48_000,
			sourceOut: 320 / 48_000,
			timelineStart: 0,
			effects: { ...DEFAULT_EFFECTS, volumeDb: -6, reverse: true },
		};
		const first = await requestSharedSegmentRender(
			source as unknown as AudioBuffer,
			segment,
		);
		expect(first?.pcm.frames).toBe(256);
		expect(first?.pcm.interleaved[0]).toBeCloseTo(
			(source.channels[0]?.[319] ?? 0) * 10 ** -0.3,
			5,
		);
		const second = await requestSharedSegmentRender(
			source as unknown as AudioBuffer,
			segment,
		);
		expect(second).toBe(first);
		const geometry = await requestSharedSegmentPreprocessedPcm(
			source as unknown as AudioBuffer,
			segment,
		);

		await requestSharedSegmentRender(source as unknown as AudioBuffer, {
			...segment,
			effects: { ...segment.effects, volumeDb: -7 },
		});
		expect(
			await requestSharedSegmentPreprocessedPcm(
				source as unknown as AudioBuffer,
				{ ...segment, effects: { ...segment.effects, volumeDb: -7 } },
			),
		).toBe(geometry);
		const rerendered = await requestSharedSegmentRender(
			source as unknown as AudioBuffer,
			segment,
		);
		expect(rerendered).not.toBe(first);
	});

	test("replaces an obsolete queued edit for the same segment", async () => {
		await warmSharedDsp();
		const source = new TestAudioBuffer(2, 16_384, 48_000);
		const left = source.channels[0];
		const right = source.channels[1];
		if (!left || !right) throw new Error("stereo fixture was not allocated");
		for (let frame = 0; frame < source.length; frame += 1) {
			left[frame] = Math.sin((Math.PI * 2 * frame) / 137) * 0.2;
			right[frame] = Math.sin((Math.PI * 2 * frame) / 251) * 0.2;
		}
		const segment: TimelineSegment = {
			id: "worker-coalescing-segment",
			track: 0,
			source: "clip",
			sourceId: "worker-coalescing-source",
			sourceIn: 0,
			sourceOut: source.length / source.sampleRate,
			timelineStart: 0,
			effects: { ...DEFAULT_EFFECTS, pitchCents: 400, rate: 0.85 },
		};
		const active = requestSharedSegmentRender(
			source as unknown as AudioBuffer,
			segment,
		);
		const obsolete = requestSharedSegmentRender(
			source as unknown as AudioBuffer,
			{
				...segment,
				effects: { ...segment.effects, volumeDb: -1 },
			},
		);
		const newest = requestSharedSegmentRender(
			source as unknown as AudioBuffer,
			{
				...segment,
				effects: { ...segment.effects, volumeDb: -2 },
			},
		);

		expect(await obsolete).toBeNull();
		// The first request's geometry may already be active, but its now-obsolete
		// downstream render is still discarded before it consumes another pass.
		expect(await active).toBeNull();
		expect(await newest).not.toBeNull();
	});

	test("renders trimmed stereo PCM, peaks, and tails through WASM", async () => {
		await initializeSharedDspWorkerRuntime();
		const sampleRate = 48_000;
		const left = new Float32Array(256);
		const right = new Float32Array(256);
		for (let frame = 0; frame < left.length; frame += 1) {
			left[frame] = (frame + 64) / 512;
			right[frame] = -(frame + 64) / 512;
		}
		const render = await renderThroughSplit(1, sampleRate, left, right, {
			...DEFAULT_EFFECTS,
			volumeDb: -6,
			reverse: true,
		});

		expect(render.pcm.frames).toBe(256);
		expect(render.pcm.channels).toBe(2);
		expect(render.pcm.interleaved[0]).toBeCloseTo(
			(left[255] ?? 0) * 10 ** (-6 / 20),
			5,
		);
		expect(render.peaks.min).toHaveLength(256);
		expect(render.peaks.max).toHaveLength(256);

		const tailed = await renderThroughSplit(2, sampleRate, left, right, {
			...DEFAULT_EFFECTS,
			volumeDb: -6,
			reverse: true,
			tailSeconds: 0.001,
		});
		expect(tailed.pcm.frames).toBe(256 + 48);
	});

	test("supports the complete advanced effect chain", async () => {
		const input = new Float32Array(512);
		for (let frame = 0; frame < input.length; frame += 1) {
			input[frame] = Math.sin((Math.PI * 2 * frame) / 37) * 0.25;
		}
		const effects = {
			...DEFAULT_EFFECTS,
			pitchCents: 350,
			rate: 0.9,
			tailSeconds: 0.002,
			distortionAmount: 0.7,
			distortionWet: 0.4,
			delaySeconds: 0.001,
			delayFeedback: 0.2,
			delayWet: 0.3,
			compressorEnabled: true,
			chorusEnabled: true,
			reverbEnabled: true,
			reverbDecaySeconds: 0.1,
			reverbPreDelaySeconds: 0,
			reverbWet: 0.25,
			reverbSeed: 42,
		};
		const render = await renderThroughSplit(
			3,
			48_000,
			input,
			input.slice(),
			effects,
		);
		expect(render.pcm.frames).toBe(
			Math.round(input.length / effects.rate) + 96,
		);
		expect(render.pcm.interleaved.some((sample) => sample !== 0)).toBe(true);

		const interleaved = new Float32Array(input.length * 2);
		for (let frame = 0; frame < input.length; frame += 1) {
			interleaved[frame * 2] = input[frame] ?? 0;
			interleaved[frame * 2 + 1] = input[frame] ?? 0;
		}
		const processor = new WasmSegmentProcessor(48_000, 2);
		try {
			expect(processor.set_effect_config(sharedDspEffectConfig(effects))).toBe(
				true,
			);
			const monolithic = processor.render_clip_interleaved(interleaved);
			expect(render.pcm.interleaved).toEqual(monolithic);
		} finally {
			processor.free();
		}
	});

	test("uses a complete versioned effect configuration", async () => {
		await initializeSharedDspWorkerRuntime();
		const processor = new WasmSegmentProcessor(48_000, 2);
		try {
			const config = sharedDspEffectConfig(DEFAULT_EFFECTS);
			expect(config.version).toBe(2);
			expect(processor.set_effect_config(config)).toBe(true);
			expect(
				processor.set_effect_config({ ...config, version: config.version + 1 }),
			).toBe(false);
			expect(
				processor.set_effect_config({
					version: config.version,
					effects: { volumeDb: 0 },
				}),
			).toBe(false);
		} finally {
			processor.free();
		}
	});
});
