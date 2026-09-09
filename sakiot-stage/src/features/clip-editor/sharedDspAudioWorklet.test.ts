import { describe, expect, test } from "bun:test";
import {
	type WorkletLoaders,
	type WorkletResources,
	warmSharedDspAudioWorklet,
} from "./sharedDspAudioWorklet";

/** The worklet only ever touches `audioWorklet.addModule` on the context. */
function fakeContext(): AudioContext {
	return {
		audioWorklet: { addModule: async () => {} },
	} as unknown as AudioContext;
}

function fakeWasm(): WebAssembly.Module {
	// The smallest valid module: the 8-byte header with no sections.
	return new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]));
}

function loaders(overrides: Partial<WorkletLoaders> = {}): WorkletLoaders & {
	addModuleCalls: number;
	compileCalls: number;
} {
	const state = {
		addModuleCalls: 0,
		compileCalls: 0,
		addModule: async () => {
			state.addModuleCalls += 1;
		},
		compileWasm: async () => {
			state.compileCalls += 1;
			return fakeWasm();
		},
		...overrides,
	};
	return state as WorkletLoaders & {
		addModuleCalls: number;
		compileCalls: number;
	};
}

describe("warmSharedDspAudioWorklet", () => {
	test("recovers after a transient failure", async () => {
		const context = fakeContext();
		let compileAttempts = 0;
		const testLoaders = loaders({
			compileWasm: async () => {
				compileAttempts += 1;
				if (compileAttempts === 1) throw new Error("transient network error");
				return fakeWasm();
			},
		});

		const failed = await warmSharedDspAudioWorklet(context, testLoaders);
		expect(failed).toBeNull();

		// The rejected attempt must not be cached: one transient failure used
		// to disable streaming effects for the rest of the session.
		const recovered = await warmSharedDspAudioWorklet(context, testLoaders);
		expect(recovered).not.toBeNull();
		expect(recovered?.wasmModule).toBeInstanceOf(WebAssembly.Module);
	});

	test("concurrent callers share a single attempt", async () => {
		const context = fakeContext();
		let compileCalls = 0;
		let release!: () => void;
		const gate = new Promise<void>((resolve) => {
			release = resolve;
		});
		const testLoaders = loaders({
			compileWasm: async () => {
				compileCalls += 1;
				await gate;
				return fakeWasm();
			},
		});

		const first = warmSharedDspAudioWorklet(context, testLoaders);
		const second = warmSharedDspAudioWorklet(context, testLoaders);
		const third = warmSharedDspAudioWorklet(context, testLoaders);
		release();
		const results = await Promise.all([first, second, third]);

		expect(compileCalls).toBe(1);
		expect(testLoaders.addModuleCalls).toBe(1);
		expect(results.every((result) => result !== null)).toBe(true);
		expect(results[0]).toBe(results[1]);
	});

	test("does not re-register the processor when only the WASM failed", async () => {
		const context = fakeContext();
		let compileAttempts = 0;
		const testLoaders = loaders({
			compileWasm: async () => {
				compileAttempts += 1;
				if (compileAttempts === 1) throw new Error("wasm compile failed");
				return fakeWasm();
			},
		});

		await warmSharedDspAudioWorklet(context, testLoaders);
		await warmSharedDspAudioWorklet(context, testLoaders);

		expect(testLoaders.addModuleCalls).toBe(1);
		expect(compileAttempts).toBe(2);
	});

	test("caches a successful attempt per context", async () => {
		const context = fakeContext();
		const testLoaders = loaders();

		const first = await warmSharedDspAudioWorklet(context, testLoaders);
		const second = await warmSharedDspAudioWorklet(context, testLoaders);

		expect(testLoaders.compileCalls).toBe(1);
		expect(first).toBe(second);
	});

	test("stops retrying after repeated failures", async () => {
		const context = fakeContext();
		const testLoaders = loaders({
			compileWasm: async () => {
				throw new Error("bundle missing");
			},
		});

		for (let attempt = 0; attempt < 3; attempt += 1) {
			expect(await warmSharedDspAudioWorklet(context, testLoaders)).toBeNull();
		}
		const attemptsAfterCap = testLoaders.compileCalls;

		// The cap keeps a permanently broken bundle from being re-fetched on
		// every playback.
		expect(await warmSharedDspAudioWorklet(context, testLoaders)).toBeNull();
		expect(testLoaders.compileCalls).toBe(attemptsAfterCap);
	});

	test("a fresh context gets its own attempt budget", async () => {
		const broken = fakeContext();
		const brokenLoaders = loaders({
			compileWasm: async () => {
				throw new Error("bundle missing");
			},
		});
		for (let attempt = 0; attempt < 4; attempt += 1) {
			await warmSharedDspAudioWorklet(broken, brokenLoaders);
		}

		const healthy = fakeContext();
		const healthyLoaders = loaders();
		const resources: WorkletResources | null = await warmSharedDspAudioWorklet(
			healthy,
			healthyLoaders,
		);
		expect(resources).not.toBeNull();
	});
});
