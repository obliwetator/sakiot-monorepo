import type { SegmentEffects } from "./model";
import { sharedDspStreamingEffectConfig } from "./sharedDspConfig";

const dspWasmUrl = new URL(
	"../../../../sakiot-DSP/pkg/sakiot_dsp_bg.wasm",
	import.meta.url,
).href;
const dspWorkletUrl = new URL(
	"../../../../sakiot-DSP/pkg/sakiot-dsp-worklet.bundle.js",
	import.meta.url,
).href;

export interface WorkletResources {
	wasmModule: WebAssembly.Module;
}

const resourcesByContext = new WeakMap<
	AudioContext,
	Promise<WorkletResources | null>
>();

/** Load the bundled processor and compile the shared WASM once per context. */
export function warmSharedDspAudioWorklet(
	context: AudioContext,
): Promise<WorkletResources | null> {
	const existing = resourcesByContext.get(context);
	if (existing) return existing;
	const loading = Promise.all([
		context.audioWorklet.addModule(dspWorkletUrl),
		compileWasmModule(dspWasmUrl),
	])
		.then(([, wasmModule]) => ({ wasmModule }))
		.catch(() => null);
	resourcesByContext.set(context, loading);
	return loading;
}

export function createSharedDspAudioWorkletNode(
	context: AudioContext,
	resources: WorkletResources,
	effects: SegmentEffects,
	startTime: number,
): AudioWorkletNode {
	return new AudioWorkletNode(context, "sakiot-dsp", {
		numberOfInputs: 1,
		numberOfOutputs: 1,
		outputChannelCount: [2],
		processorOptions: {
			wasmModule: resources.wasmModule,
			effects: sharedDspStreamingEffectConfig(effects),
			startFrame: Math.round(startTime * context.sampleRate),
		},
	});
}

export function updateSharedDspAudioWorkletNode(
	node: AudioWorkletNode,
	effects: SegmentEffects,
) {
	node.port.postMessage({
		type: "effects",
		effects: sharedDspStreamingEffectConfig(effects),
	});
}

async function compileWasmModule(url: string): Promise<WebAssembly.Module> {
	const response = await fetch(url);
	if (!response.ok) {
		throw new Error(`Shared DSP WASM request failed (${response.status})`);
	}
	if (typeof WebAssembly.compileStreaming === "function") {
		try {
			return await WebAssembly.compileStreaming(
				Promise.resolve(response.clone()),
			);
		} catch {
			// Development servers or proxies may omit application/wasm.
		}
	}
	return WebAssembly.compile(await response.arrayBuffer());
}
