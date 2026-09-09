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

/** How the processor and its WASM reach a context. Injectable for tests. */
export interface WorkletLoaders {
	addModule(context: AudioContext): Promise<void>;
	compileWasm(): Promise<WebAssembly.Module>;
}

const defaultLoaders: WorkletLoaders = {
	addModule: (context) => context.audioWorklet.addModule(dspWorkletUrl),
	compileWasm: () => compileWasmModule(dspWasmUrl),
};

/**
 * Consecutive failures after which a context stops retrying. The processor
 * bundle is a static asset: once it has failed this many times it is not going
 * to appear, and re-fetching it on every playback would only slow the editor.
 */
const MAX_LOAD_ATTEMPTS = 3;

interface ContextWorkletState {
	/** `addModule` resolves once per context; retries must not repeat it. */
	moduleRegistered: boolean;
	/** The shared in-flight or completed attempt. */
	loading: Promise<WorkletResources | null> | null;
	failedAttempts: number;
}

const stateByContext = new WeakMap<AudioContext, ContextWorkletState>();

function stateFor(context: AudioContext): ContextWorkletState {
	let state = stateByContext.get(context);
	if (!state) {
		state = { moduleRegistered: false, loading: null, failedAttempts: 0 };
		stateByContext.set(context, state);
	}
	return state;
}

/** Load the bundled processor and compile the shared WASM once per context. */
export function warmSharedDspAudioWorklet(
	context: AudioContext,
	loaders: WorkletLoaders = defaultLoaders,
): Promise<WorkletResources | null> {
	const state = stateFor(context);
	// Concurrent callers share one attempt instead of racing two fetches.
	if (state.loading) return state.loading;
	if (state.failedAttempts >= MAX_LOAD_ATTEMPTS) return Promise.resolve(null);

	const loading = loadWorkletResources(context, state, loaders).then(
		(resources) => {
			// A failed attempt must not stay cached: one transient network error
			// would otherwise disable streaming effects for the whole session.
			if (resources === null) {
				state.failedAttempts += 1;
				state.loading = null;
			} else {
				state.failedAttempts = 0;
			}
			return resources;
		},
	);
	state.loading = loading;
	return loading;
}

async function loadWorkletResources(
	context: AudioContext,
	state: ContextWorkletState,
	loaders: WorkletLoaders,
): Promise<WorkletResources | null> {
	try {
		// Registering the same processor twice throws in some browsers, so a
		// retry after a WASM failure only recompiles the module.
		if (!state.moduleRegistered) {
			await loaders.addModule(context);
			state.moduleRegistered = true;
		}
		return { wasmModule: await loaders.compileWasm() };
	} catch {
		return null;
	}
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
