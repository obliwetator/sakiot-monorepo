import { describe, expect, test } from "bun:test";
import { WasmSegmentProcessor } from "../../../../sakiot-DSP/pkg/sakiot_dsp.js";
import { DEFAULT_EFFECTS, type TimelineSegment } from "./model";
import {
	renderSharedSegment,
	renderSharedSegmentPcm,
	sharedDspEffectConfig,
	warmSharedDsp,
} from "./sharedDsp";

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

describe("shared browser DSP", () => {
	test("renders and caches a trimmed reversed segment through WASM", async () => {
		await warmSharedDsp();
		const sampleRate = 48_000;
		const source = new TestAudioBuffer(2, 512, sampleRate);
		const left = source.channels[0];
		const right = source.channels[1];
		if (!left || !right) throw new Error("stereo fixture was not allocated");
		for (let frame = 0; frame < source.length; frame += 1) {
			left[frame] = frame / source.length;
			right[frame] = -frame / source.length;
		}
		const context = {
			createBuffer: (channels: number, length: number, rate: number) =>
				new TestAudioBuffer(channels, length, rate),
		} as unknown as AudioContext;
		const segment: TimelineSegment = {
			id: "wasm-segment",
			track: 0,
			source: "clip",
			sourceId: "source",
			sourceIn: 64 / sampleRate,
			sourceOut: 320 / sampleRate,
			timelineStart: 0,
			effects: { ...DEFAULT_EFFECTS, volumeDb: -6, reverse: true },
		};

		const rendered = renderSharedSegment(
			context,
			source as unknown as AudioBuffer,
			segment,
		);
		expect(rendered).not.toBeNull();
		expect(rendered?.length).toBe(256);
		expect(rendered?.numberOfChannels).toBe(2);
		expect(rendered?.getChannelData(0)[0]).toBeCloseTo(
			(left[319] ?? 0) * 10 ** (-6 / 20),
			5,
		);
		expect(
			renderSharedSegment(context, source as unknown as AudioBuffer, segment),
		).toBe(rendered);
		const pcm = renderSharedSegmentPcm(
			source as unknown as AudioBuffer,
			segment,
		);
		expect(pcm?.frames).toBe(256);
		expect(
			renderSharedSegmentPcm(source as unknown as AudioBuffer, segment),
		).toBe(pcm);

		const tailed = renderSharedSegment(
			context,
			source as unknown as AudioBuffer,
			{
				...segment,
				effects: { ...segment.effects, tailSeconds: 0.001 },
			},
		);
		expect(tailed?.length).toBe(256 + 48);

		const allEffects: TimelineSegment = {
			...segment,
			effects: {
				...segment.effects,
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
			},
		};
		const processed = renderSharedSegment(
			context,
			source as unknown as AudioBuffer,
			allEffects,
		);
		expect(processed).not.toBeNull();
		expect(processed).not.toBe(rendered);
		expect(processed?.getChannelData(0)[0]).not.toBeCloseTo(
			rendered?.getChannelData(0)[0] ?? 0,
			6,
		);
	});

	test("uses a complete versioned effect configuration", async () => {
		await warmSharedDsp();
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
