//#region web/audio-worklet-polyfills.js
globalThis.TextDecoder ??= class WorkletTextDecoder {
	decode(input = new Uint8Array()) {
		const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
		let output = "";
		for (let index = 0; index < bytes.length;) {
			const first = bytes[index++] ?? 0;
			if (first < 128) {
				output += String.fromCharCode(first);
				continue;
			}
			const second = bytes[index++] ?? 0;
			if (first < 224) {
				output += String.fromCharCode((first & 31) << 6 | second & 63);
				continue;
			}
			const third = bytes[index++] ?? 0;
			if (first < 240) {
				output += String.fromCharCode((first & 15) << 12 | (second & 63) << 6 | third & 63);
				continue;
			}
			const fourth = bytes[index++] ?? 0;
			const adjusted = ((first & 7) << 18 | (second & 63) << 12 | (third & 63) << 6 | fourth & 63) - 65536;
			output += String.fromCharCode(55296 | adjusted >> 10, 56320 | adjusted & 1023);
		}
		return output;
	}
};
//#endregion
//#region pkg/sakiot_dsp.js
/**
* Copy-based prototype boundary. A production AudioWorklet may use the
* WASM linear memory directly after profiling this simpler version.
*/
var WasmSegmentProcessor = class {
	__destroy_into_raw() {
		const ptr = this.__wbg_ptr;
		this.__wbg_ptr = 0;
		WasmSegmentProcessorFinalization.unregister(this);
		return ptr;
	}
	free() {
		const ptr = this.__destroy_into_raw();
		wasm.__wbg_wasmsegmentprocessor_free(ptr, 0);
	}
	/**
	* @param {number} sample_rate
	* @param {number} channels
	*/
	constructor(sample_rate, channels) {
		this.__wbg_ptr = wasm.wasmsegmentprocessor_new(sample_rate, channels);
		WasmSegmentProcessorFinalization.register(this, this.__wbg_ptr, this);
		return this;
	}
	/**
	* @param {Float32Array} samples
	* @returns {boolean}
	*/
	process_interleaved(samples) {
		var ptr0 = passArrayF32ToWasm0(samples, wasm.__wbindgen_malloc);
		var len0 = WASM_VECTOR_LEN;
		return wasm.wasmsegmentprocessor_process_interleaved(this.__wbg_ptr, ptr0, len0, samples) !== 0;
	}
	/**
	* Offline clip path for length-changing rate/pitch transforms and
	* frame-order reverse. The returned buffer is newly allocated.
	* @param {Float32Array} samples
	* @returns {Float32Array}
	*/
	render_clip_interleaved(samples) {
		const ptr0 = passArrayF32ToWasm0(samples, wasm.__wbindgen_malloc);
		const len0 = WASM_VECTOR_LEN;
		const ret = wasm.wasmsegmentprocessor_render_clip_interleaved(this.__wbg_ptr, ptr0, len0);
		var v2 = getArrayF32FromWasm0(ret[0], ret[1]).slice();
		wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
		return v2;
	}
	reset() {
		wasm.wasmsegmentprocessor_reset(this.__wbg_ptr);
	}
	/**
	* Apply a complete, versioned JavaScript configuration object. The
	* boundary intentionally rejects missing fields or unknown versions
	* instead of silently mixing schemas.
	* @param {any} config
	* @returns {boolean}
	*/
	set_effect_config(config) {
		return wasm.wasmsegmentprocessor_set_effect_config(this.__wbg_ptr, config) !== 0;
	}
};
if (Symbol.dispose) WasmSegmentProcessor.prototype[Symbol.dispose] = WasmSegmentProcessor.prototype.free;
function __wbg_get_imports() {
	const import0 = {
		__proto__: null,
		__wbg___wbindgen_boolean_get_1a45e2c38d4d41b9: function(arg0) {
			const v = arg0;
			const ret = typeof v === "boolean" ? v : void 0;
			return isLikeNone(ret) ? 16777215 : ret ? 1 : 0;
		},
		__wbg___wbindgen_copy_to_typed_array_7a3f7b938f93cf12: function(arg0, arg1, arg2) {
			new Uint8Array(arg2.buffer, arg2.byteOffset, arg2.byteLength).set(getArrayU8FromWasm0(arg0, arg1));
		},
		__wbg___wbindgen_is_object_56732c2bc353f41d: function(arg0) {
			const val = arg0;
			return typeof val === "object" && val !== null;
		},
		__wbg___wbindgen_number_get_9bb1761122181af2: function(arg0, arg1) {
			const obj = arg1;
			const ret = typeof obj === "number" ? obj : void 0;
			getDataViewMemory0().setFloat64(arg0 + 8, isLikeNone(ret) ? 0 : ret, true);
			getDataViewMemory0().setInt32(arg0 + 0, !isLikeNone(ret), true);
		},
		__wbg___wbindgen_throw_1506f2235d1bdba0: function(arg0, arg1) {
			throw new Error(getStringFromWasm0(arg0, arg1));
		},
		__wbg_get_de6a0f7d4d18a304: function() {
			return handleError(function(arg0, arg1) {
				return Reflect.get(arg0, arg1);
			}, arguments);
		},
		__wbindgen_cast_0000000000000001: function(arg0, arg1) {
			return getStringFromWasm0(arg0, arg1);
		},
		__wbindgen_init_externref_table: function() {
			const table = wasm.__wbindgen_externrefs;
			const offset = table.grow(4);
			table.set(0, void 0);
			table.set(offset + 0, void 0);
			table.set(offset + 1, null);
			table.set(offset + 2, true);
			table.set(offset + 3, false);
		}
	};
	return {
		__proto__: null,
		"./sakiot_dsp_bg.js": import0
	};
}
const WasmSegmentProcessorFinalization = typeof FinalizationRegistry === "undefined" ? {
	register: () => {},
	unregister: () => {}
} : new FinalizationRegistry((ptr) => wasm.__wbg_wasmsegmentprocessor_free(ptr, 1));
function addToExternrefTable0(obj) {
	const idx = wasm.__externref_table_alloc();
	wasm.__wbindgen_externrefs.set(idx, obj);
	return idx;
}
function getArrayF32FromWasm0(ptr, len) {
	ptr = ptr >>> 0;
	return getFloat32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}
function getArrayU8FromWasm0(ptr, len) {
	ptr = ptr >>> 0;
	return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}
let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
	if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || cachedDataViewMemory0.buffer.detached === void 0 && cachedDataViewMemory0.buffer !== wasm.memory.buffer) cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
	return cachedDataViewMemory0;
}
let cachedFloat32ArrayMemory0 = null;
function getFloat32ArrayMemory0() {
	if (cachedFloat32ArrayMemory0 === null || cachedFloat32ArrayMemory0.byteLength === 0) cachedFloat32ArrayMemory0 = new Float32Array(wasm.memory.buffer);
	return cachedFloat32ArrayMemory0;
}
function getStringFromWasm0(ptr, len) {
	return decodeText(ptr >>> 0, len);
}
let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
	if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
	return cachedUint8ArrayMemory0;
}
function handleError(f, args) {
	try {
		return f.apply(this, args);
	} catch (e) {
		const idx = addToExternrefTable0(e);
		wasm.__wbindgen_exn_store(idx);
	}
}
function isLikeNone(x) {
	return x === void 0 || x === null;
}
function passArrayF32ToWasm0(arg, malloc) {
	const ptr = malloc(arg.length * 4, 4) >>> 0;
	getFloat32ArrayMemory0().set(arg, ptr / 4);
	WASM_VECTOR_LEN = arg.length;
	return ptr;
}
let cachedTextDecoder = new TextDecoder("utf-8", {
	ignoreBOM: true,
	fatal: true
});
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
	numBytesDecoded += len;
	if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
		cachedTextDecoder = new TextDecoder("utf-8", {
			ignoreBOM: true,
			fatal: true
		});
		cachedTextDecoder.decode();
		numBytesDecoded = len;
	}
	return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}
let WASM_VECTOR_LEN = 0, wasm;
function __wbg_finalize_init(instance, module) {
	wasm = instance.exports;
	cachedDataViewMemory0 = null;
	cachedFloat32ArrayMemory0 = null;
	cachedUint8ArrayMemory0 = null;
	wasm.__wbindgen_start();
	return wasm;
}
function initSync(module) {
	if (wasm !== void 0) return wasm;
	if (module !== void 0) if (Object.getPrototypeOf(module) === Object.prototype) ({module} = module);
	else console.warn("using deprecated parameters for `initSync()`; pass a single object instead");
	const imports = __wbg_get_imports();
	if (!(module instanceof WebAssembly.Module)) module = new WebAssembly.Module(module);
	return __wbg_finalize_init(new WebAssembly.Instance(module, imports), module);
}
//#endregion
//#region web/sakiot-dsp-worklet.js
let wasmInitialized = false;
var SakiotDspProcessor = class extends AudioWorkletProcessor {
	constructor(options) {
		super();
		this.channels = options.outputChannelCount?.[0] ?? 2;
		this.effects = {
			volumeDb: 0,
			pitchCents: 0,
			rate: 1,
			tailSeconds: 0,
			bassDb: 0,
			midDb: 0,
			trebleDb: 0,
			distortionAmount: .4,
			distortionWet: 0,
			delaySeconds: .25,
			delayFeedback: .125,
			delayWet: 0,
			compressorEnabled: false,
			compressorThresholdDb: -24,
			compressorKneeDb: 30,
			compressorRatio: 12,
			compressorAttackSeconds: .003,
			compressorReleaseSeconds: .25,
			chorusEnabled: false,
			chorusFrequencyHz: 1.5,
			chorusDelayMs: 3.5,
			chorusDepth: .7,
			chorusSpreadDegrees: 180,
			chorusFeedback: 0,
			chorusWet: .5,
			reverbEnabled: false,
			reverbDecaySeconds: 1.5,
			reverbPreDelaySeconds: .01,
			reverbWet: 1,
			reverbSeed: 1396788041,
			reverse: false,
			...options.processorOptions?.effects
		};
		this.captureRemaining = options.processorOptions?.captureFrames ?? 0;
		this.captureStarted = false;
		this.deferUntilSignal = options.processorOptions?.deferUntilSignal ?? false;
		this.startFrame = options.processorOptions?.startFrame;
		this.processingStarted = !this.deferUntilSignal && this.startFrame === void 0;
		const wasmModule = options.processorOptions?.wasmModule;
		if (!(wasmModule instanceof WebAssembly.Module)) throw new TypeError("sakiot-dsp requires a precompiled WebAssembly.Module");
		if (!wasmInitialized) {
			initSync({ module: wasmModule });
			wasmInitialized = true;
		}
		this.processor = new WasmSegmentProcessor(sampleRate, this.channels);
		this.port.onmessage = (event) => {
			if (event.data?.type !== "effects") return;
			this.effects = {
				...this.effects,
				...event.data.effects
			};
			this.port.postMessage({
				type: "effects-applied",
				ok: this.applyEffects()
			});
		};
		this.port.postMessage({
			type: "effects-applied",
			ok: this.applyEffects()
		});
	}
	applyEffects() {
		if (!this.processor) return;
		return this.processor.set_effect_config({
			version: 2,
			effects: this.effects
		});
	}
	process(inputs, outputs) {
		const input = inputs[0];
		const output = outputs[0];
		if (!input?.length) {
			for (const channel of output) channel.fill(0);
			return true;
		}
		let processFromFrame = 0;
		if (!this.processingStarted) {
			if (this.startFrame !== void 0) {
				processFromFrame = Math.max(0, this.startFrame - currentFrame);
				if (processFromFrame >= (output[0]?.length ?? 0)) {
					for (const channel of output) channel.fill(0);
					return true;
				}
			} else if (!input.some((channel) => channel.some((sample) => sample !== 0))) {
				for (const channel of output) channel.fill(0);
				return true;
			}
			if (processFromFrame > 0) for (const channel of output) channel.fill(0);
			this.processor.reset();
			this.processingStarted = true;
		}
		const frames = output[0]?.length ?? 0;
		const processingFrames = frames - processFromFrame;
		const interleaved = new Float32Array(processingFrames * this.channels);
		for (let frame = processFromFrame; frame < frames; frame += 1) for (let channel = 0; channel < this.channels; channel += 1) interleaved[(frame - processFromFrame) * this.channels + channel] = input[channel]?.[frame] ?? input[0]?.[frame] ?? 0;
		this.processor.process_interleaved(interleaved);
		for (let frame = processFromFrame; frame < frames; frame += 1) for (let channel = 0; channel < this.channels; channel += 1) output[channel][frame] = interleaved[(frame - processFromFrame) * this.channels + channel];
		if (this.captureRemaining > 0) {
			if (!this.captureStarted) this.captureStarted = input.some((channel) => channel.some((sample) => sample !== 0));
			if (this.captureStarted) {
				const captureFrames = Math.min(frames, this.captureRemaining);
				const channels = output.map((channel) => channel.slice(0, captureFrames));
				this.port.postMessage({
					type: "capture",
					channels
				}, channels.map((channel) => channel.buffer));
				this.captureRemaining -= captureFrames;
			}
		}
		return true;
	}
};
registerProcessor("sakiot-dsp", SakiotDspProcessor);
//#endregion
