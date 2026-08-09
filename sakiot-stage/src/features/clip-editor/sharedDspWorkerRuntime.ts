import initDsp, {
	WasmSegmentProcessor,
} from "../../../../sakiot-DSP/pkg/sakiot_dsp.js";
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
	const interleaved = new Float32Array(frameCount * OUTPUT_CHANNELS);
	for (let frame = 0; frame < frameCount; frame += 1) {
		interleaved[frame * OUTPUT_CHANNELS] = request.left[frame] ?? 0;
		interleaved[frame * OUTPUT_CHANNELS + 1] = request.right[frame] ?? 0;
	}

	const processor = new WasmSegmentProcessor(
		request.sampleRate,
		OUTPUT_CHANNELS,
	);
	try {
		if (
			!processor.set_effect_config(
				sharedDspEffectConfig(sharedDspPreprocessEffectConfig(request.effects)),
			)
		) {
			throw new Error("Shared DSP rejected the complete effect configuration");
		}
		const rendered = processor.render_clip_interleaved(interleaved);
		if (rendered.length === 0) {
			throw new Error("Shared DSP returned an empty render");
		}
		return {
			channels: OUTPUT_CHANNELS,
			sampleRate: request.sampleRate,
			frames: Math.floor(rendered.length / OUTPUT_CHANNELS),
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
		if (!processor.process_interleaved(request.pcm.interleaved)) {
			throw new Error("Shared DSP could not process the preprocessed clip");
		}
		return {
			pcm: request.pcm,
			peaks: waveformEnvelopeFromPcm(request.pcm),
		};
	} finally {
		processor.free();
	}
}
