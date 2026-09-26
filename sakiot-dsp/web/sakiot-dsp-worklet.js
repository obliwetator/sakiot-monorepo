// Prototype AudioWorklet wrapper for the output of:
//   wasm-pack build --target web --features wasm
//
// Bundle/deployment integration is intentionally deferred. This file shows
// the real-time boundary and uses the exact same SegmentProcessor as native.
import "./audio-worklet-polyfills.js";
import { initSync, WasmSegmentProcessor } from "../pkg/sakiot_dsp.js";

let wasmInitialized = false;

class SakiotDspProcessor extends AudioWorkletProcessor {
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
			distortionAmount: 0.4,
			distortionWet: 0,
			delaySeconds: 0.25,
			delayFeedback: 0.125,
			delayWet: 0,
			compressorEnabled: false,
			compressorThresholdDb: -24,
			compressorKneeDb: 30,
			compressorRatio: 12,
			compressorAttackSeconds: 0.003,
			compressorReleaseSeconds: 0.25,
			chorusEnabled: false,
			chorusFrequencyHz: 1.5,
			chorusDelayMs: 3.5,
			chorusDepth: 0.7,
			chorusSpreadDegrees: 180,
			chorusFeedback: 0,
			chorusWet: 0.5,
			reverbEnabled: false,
			reverbDecaySeconds: 1.5,
			reverbPreDelaySeconds: 0.01,
			reverbWet: 1,
			reverbSeed: 0x53414b49,
			reverse: false,
			...options.processorOptions?.effects,
		};
		this.captureRemaining = options.processorOptions?.captureFrames ?? 0;
		this.captureStarted = false;
		this.deferUntilSignal = options.processorOptions?.deferUntilSignal ?? false;
		this.startFrame = options.processorOptions?.startFrame;
		this.processingStarted =
			!this.deferUntilSignal && this.startFrame === undefined;
		const wasmModule = options.processorOptions?.wasmModule;
		if (!(wasmModule instanceof WebAssembly.Module)) {
			throw new TypeError("sakiot-dsp requires a precompiled WebAssembly.Module");
		}
		if (!wasmInitialized) {
			initSync({ module: wasmModule });
			wasmInitialized = true;
		}
		this.processor = new WasmSegmentProcessor(sampleRate, this.channels);
		this.port.onmessage = (event) => {
			if (event.data?.type !== "effects") return;
			this.effects = { ...this.effects, ...event.data.effects };
			this.port.postMessage({ type: "effects-applied", ok: this.applyEffects() });
		};
		this.port.postMessage({ type: "effects-applied", ok: this.applyEffects() });
	}

	applyEffects() {
		if (!this.processor) return;
		return this.processor.set_effect_config({
			version: 2,
			effects: this.effects,
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
			if (this.startFrame !== undefined) {
				processFromFrame = Math.max(0, this.startFrame - currentFrame);
				if (processFromFrame >= (output[0]?.length ?? 0)) {
					for (const channel of output) channel.fill(0);
					return true;
				}
			} else {
				const hasSignal = input.some((channel) =>
					channel.some((sample) => sample !== 0),
				);
				if (!hasSignal) {
					for (const channel of output) channel.fill(0);
					return true;
				}
			}
			if (processFromFrame > 0) {
				for (const channel of output) channel.fill(0);
			}
			this.processor.reset();
			this.processingStarted = true;
		}

		// The first spike keeps the WASM boundary simple. Production should
		// profile direct linear-memory views before adopting this interleave copy.
		const frames = output[0]?.length ?? 0;
		const processingFrames = frames - processFromFrame;
		const interleaved = new Float32Array(processingFrames * this.channels);
		for (let frame = processFromFrame; frame < frames; frame += 1) {
			for (let channel = 0; channel < this.channels; channel += 1) {
				interleaved[(frame - processFromFrame) * this.channels + channel] =
					input[channel]?.[frame] ?? input[0]?.[frame] ?? 0;
			}
		}
		this.processor.process_interleaved(interleaved);
		for (let frame = processFromFrame; frame < frames; frame += 1) {
			for (let channel = 0; channel < this.channels; channel += 1) {
				output[channel][frame] =
					interleaved[(frame - processFromFrame) * this.channels + channel];
			}
		}
		if (this.captureRemaining > 0) {
			if (!this.captureStarted) {
				this.captureStarted = input.some((channel) =>
					channel.some((sample) => sample !== 0),
				);
			}
			if (this.captureStarted) {
				const captureFrames = Math.min(frames, this.captureRemaining);
				const channels = output.map((channel) => channel.slice(0, captureFrames));
				this.port.postMessage(
					{ type: "capture", channels },
					channels.map((channel) => channel.buffer),
				);
				this.captureRemaining -= captureFrames;
			}
		}
		return true;
	}
}

registerProcessor("sakiot-dsp", SakiotDspProcessor);
