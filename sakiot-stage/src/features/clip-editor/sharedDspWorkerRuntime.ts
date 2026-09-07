import initDsp, {
	WasmIncrementalRenderer,
	WasmSegmentProcessor,
} from "../../../../sakiot-DSP/pkg/sakiot_dsp.js";
import { MAX_BROWSER_RENDER_BYTES } from "./pcmBudget";
import { waveformEnvelopeFromPcm } from "./processedWaveform";
import {
	sharedDspEffectConfig,
	sharedDspPreprocessEffectConfig,
	sharedDspStreamingEffectConfig,
} from "./sharedDspConfig";
import type {
	SharedDspPcm,
	SharedDspRender,
	SharedDspWorkerPreprocessRequest,
	SharedDspWorkerProcessRequest,
} from "./sharedDspProtocol";

const OUTPUT_CHANNELS = 2;
const BLOCK_FRAMES = 4096;
let initialization: Promise<void> | null = null;

export function initializeSharedDspWorkerRuntime(): Promise<void> {
	initialization ??= initDsp().then(() => undefined);
	return initialization;
}

export async function preprocessSharedDspWorkerRequest(
	request: SharedDspWorkerPreprocessRequest,
): Promise<SharedDspPcm> {
	await initializeSharedDspWorkerRuntime();
	const frameCount = Math.min(request.left.length, request.right.length);
	const effects = sharedDspPreprocessEffectConfig(request.effects);
	const processor = new WasmIncrementalRenderer(
		request.sampleRate,
		OUTPUT_CHANNELS,
		frameCount,
		sharedDspEffectConfig({ ...effects, reverse: false }),
	);
	try {
		const outputFrames = processor.output_frames();
		if (outputFrames * OUTPUT_CHANNELS * 4 > MAX_BROWSER_RENDER_BYTES) {
			throw new Error(
				"Preview audio exceeds the browser's 64 MiB segment limit. Shorten the segment or increase its rate; server exports support longer audio.",
			);
		}
		const rendered = new Float32Array(outputFrames * OUTPUT_CHANNELS);
		const block = new Float32Array(BLOCK_FRAMES * OUTPUT_CHANNELS);
		let offset = 0;
		for (let base = 0; base < frameCount; base += BLOCK_FRAMES) {
			const count = Math.min(BLOCK_FRAMES, frameCount - base);
			for (let frame = 0; frame < count; frame += 1) {
				const source = effects.reverse
					? frameCount - 1 - base - frame
					: base + frame;
				block[frame * 2] = request.left[source] ?? 0;
				block[frame * 2 + 1] = request.right[source] ?? 0;
			}
			const output = processor.push(block.subarray(0, count * OUTPUT_CHANNELS));
			rendered.set(output, offset);
			offset += output.length;
		}
		const final = processor.finish();
		rendered.set(final, offset);
		if (offset + final.length !== rendered.length || rendered.length === 0) {
			throw new Error("Shared DSP returned an incomplete render");
		}
		return {
			channels: OUTPUT_CHANNELS,
			sampleRate: request.sampleRate,
			frames: outputFrames,
			interleaved: rendered,
		};
	} finally {
		processor.free();
	}
}

export async function processSharedDspWorkerRequest(
	request: SharedDspWorkerProcessRequest,
): Promise<SharedDspRender> {
	await initializeSharedDspWorkerRuntime();
	const processor = new WasmSegmentProcessor(
		request.pcm.sampleRate,
		request.pcm.channels,
	);
	try {
		if (
			!processor.set_effect_config(
				sharedDspEffectConfig(sharedDspStreamingEffectConfig(request.effects)),
			)
		) {
			throw new Error("Shared DSP rejected the streaming effect configuration");
		}
		for (
			let offset = 0;
			offset < request.pcm.interleaved.length;
			offset += BLOCK_FRAMES * request.pcm.channels
		) {
			if (
				!processor.process_interleaved(
					request.pcm.interleaved.subarray(
						offset,
						offset + BLOCK_FRAMES * request.pcm.channels,
					),
				)
			) {
				throw new Error("Shared DSP could not process the preprocessed clip");
			}
		}
		return {
			pcm: request.pcm,
			peaks: waveformEnvelopeFromPcm(request.pcm),
		};
	} finally {
		processor.free();
	}
}
