import {
	connect as connectTone,
	Compressor,
	Chorus,
	Distortion,
	FeedbackDelay,
	Filter,
	OfflineContext as ToneOfflineContext,
	Volume,
} from "/tone/index.js";
import initWasm, { WasmSegmentProcessor } from "/pkg/sakiot_dsp.js";

const SAMPLE_RATE = 48_000;
const CHANNELS = 2;
const EFFECT_CASES = {
	volume: { volumeDb: -3, bassDb: 0, midDb: 0, trebleDb: 0 },
	bass: { volumeDb: 0, bassDb: 5, midDb: 0, trebleDb: 0 },
	mid: { volumeDb: 0, bassDb: 0, midDb: -4, trebleDb: 0 },
	treble: { volumeDb: 0, bassDb: 0, midDb: 0, trebleDb: 2.5 },
	combined: { volumeDb: -3, bassDb: 5, midDb: -4, trebleDb: 2.5 },
	distortion: { distortionAmount: 0.7, distortionWet: 1 },
	delay: { delaySeconds: 0.125, delayFeedback: 0, delayWet: 1 },
	fractionalDelay: { delaySeconds: 0.012345, delayFeedback: 0, delayWet: 1 },
	feedbackDelay: { delaySeconds: 0.125, delayFeedback: 0.4, delayWet: 1 },
	delayMixed: {
		delaySeconds: 0.012345,
		delayFeedback: 0.25,
		delayWet: 0.35,
	},
	combinedFx: {
		volumeDb: -3,
		bassDb: 5,
		midDb: -4,
		trebleDb: 2.5,
		distortionAmount: 0.7,
		distortionWet: 1,
	},
	combinedAll: {
		volumeDb: -3,
		bassDb: 5,
		midDb: -4,
		trebleDb: 2.5,
		distortionAmount: 0.7,
		distortionWet: 1,
		delaySeconds: 0.125,
		delayFeedback: 0.4,
		delayWet: 0.35,
	},
	compressor: {
		compressorEnabled: true,
		compressorThresholdDb: -24,
		compressorKneeDb: 30,
		compressorRatio: 12,
		compressorAttackSeconds: 0.003,
		compressorReleaseSeconds: 0.25,
	},
	chorus: {
		chorusEnabled: true,
		chorusFrequencyHz: 1.5,
		chorusDelayMs: 3.5,
		chorusDepth: 0.7,
		chorusSpreadDegrees: 180,
		chorusFeedback: 0,
		chorusWet: 0.5,
	},
};

let wasmReady;

function effectsWithDefaults(effects) {
	return {
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
		...effects,
	};
}

function fixtures() {
	const frames = SAMPLE_RATE * 2;
	const impulse = Array.from({ length: CHANNELS }, () => new Float32Array(frames));
	impulse[0][0] = 0.25;
	impulse[1][17] = -0.2;
	const sweep = Array.from({ length: CHANNELS }, () => new Float32Array(frames));
	const mixed = Array.from({ length: CHANNELS }, () => new Float32Array(frames));
	let noise = 0x51a910d5;
	let sweepPhase = 0;
	for (let frame = 0; frame < frames; frame += 1) {
		const time = frame / SAMPLE_RATE;
		const progress = frame / Math.max(1, frames - 1);
		const sweepFrequency = 20 * Math.pow(20_000 / 20, progress);
		sweepPhase += (2 * Math.PI * sweepFrequency) / SAMPLE_RATE;
		noise ^= noise << 13;
		noise ^= noise >>> 17;
		noise ^= noise << 5;
		noise >>>= 0;
		const random = (noise / 0xffffffff) * 2 - 1;
		for (let channel = 0; channel < CHANNELS; channel += 1) {
			sweep[channel][frame] = Math.sin(sweepPhase + channel * 0.31) * 0.08;
			mixed[channel][frame] =
				0.08 * Math.sin(2 * Math.PI * (83 + channel * 41) * time) +
				0.06 * Math.sin(2 * Math.PI * (997 + channel * 211) * time) +
				0.04 * Math.sin(2 * Math.PI * (8_317 - channel * 379) * time) +
				0.015 * random;
		}
	}
	return { impulse, sweep, mixed };
}

async function renderWasm(input, requestedEffects) {
	wasmReady ??= initWasm();
	await wasmReady;
	const effects = effectsWithDefaults(requestedEffects);
	const frames = input[0].length;
	const interleaved = new Float32Array(frames * CHANNELS);
	for (let frame = 0; frame < frames; frame += 1) {
		for (let channel = 0; channel < CHANNELS; channel += 1) {
			interleaved[frame * CHANNELS + channel] = input[channel][frame];
		}
	}
	const processor = new WasmSegmentProcessor(SAMPLE_RATE, CHANNELS);
	processor.set_effect_config({ version: 2, effects });
	processor.process_interleaved(interleaved);
	const output = Array.from({ length: CHANNELS }, () => new Float32Array(frames));
	for (let frame = 0; frame < frames; frame += 1) {
		for (let channel = 0; channel < CHANNELS; channel += 1) {
			output[channel][frame] = interleaved[frame * CHANNELS + channel];
		}
	}
	processor.free();
	return output;
}

async function renderTone(input, requestedEffects) {
	const effects = effectsWithDefaults(requestedEffects);
	const frames = input[0].length;
	const context = new ToneOfflineContext(CHANNELS, frames / SAMPLE_RATE, SAMPLE_RATE);
	const source = context.rawContext.createBufferSource();
	const buffer = context.createBuffer(CHANNELS, frames, SAMPLE_RATE);
	for (let channel = 0; channel < CHANNELS; channel += 1) {
		buffer.copyToChannel(input[channel], channel);
	}
	source.buffer = buffer;
	const volume = new Volume({ context, volume: effects.volumeDb });
	const bass = new Filter({
		context,
		frequency: 250,
		gain: effects.bassDb,
		rolloff: -12,
		type: "lowshelf",
	});
	const mid = new Filter({
		context,
		frequency: 1_000,
		gain: effects.midDb,
		Q: 1,
		rolloff: -12,
		type: "peaking",
	});
	const treble = new Filter({
		context,
		frequency: 3_000,
		gain: effects.trebleDb,
		rolloff: -12,
		type: "highshelf",
	});
	const distortion =
		effects.distortionWet > 0
			? new Distortion({
					context,
					distortion: effects.distortionAmount,
					oversample: "none",
					wet: effects.distortionWet,
				})
			: null;
	const delay =
		effects.delayWet > 0
			? new FeedbackDelay({
					context,
					delayTime: effects.delaySeconds,
					feedback: effects.delayFeedback,
					maxDelay: Math.max(1, effects.delaySeconds),
					wet: effects.delayWet,
				})
			: null;
	const compressor = effects.compressorEnabled
		? new Compressor({
				context,
				threshold: effects.compressorThresholdDb,
				knee: effects.compressorKneeDb,
				ratio: effects.compressorRatio,
				attack: effects.compressorAttackSeconds,
				release: effects.compressorReleaseSeconds,
			})
		: null;
	const chorus = effects.chorusEnabled
		? new Chorus({
				context,
				frequency: effects.chorusFrequencyHz,
				delayTime: effects.chorusDelayMs,
				depth: effects.chorusDepth,
				spread: effects.chorusSpreadDegrees,
				feedback: effects.chorusFeedback,
				wet: effects.chorusWet,
				type: "sine",
			}).start(0)
		: null;
	const effectNodes = [volume, bass, mid, treble];
	if (distortion) effectNodes.push(distortion);
	if (delay) effectNodes.push(delay);
	if (chorus) effectNodes.push(chorus);
	if (compressor) effectNodes.push(compressor);
	effectNodes.push(context.destination);
	connectTone(source, effectNodes[0]);
	for (let index = 0; index < effectNodes.length - 1; index += 1) {
		connectTone(effectNodes[index], effectNodes[index + 1]);
	}
	source.start(0);
	const rendered = await context.render(false);
	const output = Array.from({ length: CHANNELS }, (_, channel) =>
		rendered.getChannelData(channel).slice(),
	);
	volume.dispose();
	bass.dispose();
	mid.dispose();
	treble.dispose();
	distortion?.dispose();
	delay?.dispose();
	chorus?.dispose();
	compressor?.dispose();
	return output;
}

async function renderWorklet(input, requestedEffects) {
	const effects = effectsWithDefaults(requestedEffects);
	const frames = input[0].length;
	const context = new AudioContext({ sampleRate: SAMPLE_RATE });
	if (!context.audioWorklet) {
		throw new Error("AudioContext.audioWorklet is unavailable");
	}
	await context.resume();
	const wasmModule = await WebAssembly.compileStreaming(
		fetch("/pkg/sakiot_dsp_bg.wasm"),
	);
	await context.audioWorklet.addModule("/passthrough-worklet.js");
	try {
		const probe = new AudioWorkletNode(context, "sakiot-passthrough");
		probe.disconnect();
	} catch (error) {
		throw new Error(`basic AudioWorklet registration failed: ${error}`);
	}
	await context.audioWorklet.addModule("/worklet.js");
	const source = context.createBufferSource();
	const buffer = context.createBuffer(CHANNELS, frames, SAMPLE_RATE);
	for (let channel = 0; channel < CHANNELS; channel += 1) {
		buffer.copyToChannel(input[channel], channel);
	}
	source.buffer = buffer;
	const processor = new AudioWorkletNode(context, "sakiot-dsp", {
		numberOfInputs: 1,
		numberOfOutputs: 1,
		outputChannelCount: [CHANNELS],
		processorOptions: {
			effects,
			wasmModule,
			captureFrames: frames,
			deferUntilSignal: true,
		},
	});
	const chunks = [];
	let capturedFrames = 0;
	const captured = new Promise((resolve, reject) => {
		const timeout = setTimeout(
			() => reject(new Error(`AudioWorklet captured ${capturedFrames}/${frames} frames`)),
			5_000,
		);
		processor.onprocessorerror = () => {
			clearTimeout(timeout);
			reject(new Error("AudioWorklet processor error"));
		};
		processor.port.onmessage = (event) => {
			if (event.data?.type === "effects-applied" && !event.data.ok) {
				clearTimeout(timeout);
				reject(new Error("AudioWorklet rejected the effect configuration"));
				return;
			}
			if (event.data?.type !== "capture") return;
			chunks.push(event.data.channels);
			capturedFrames += event.data.channels[0]?.length ?? 0;
			if (capturedFrames >= frames) {
				clearTimeout(timeout);
				resolve();
			}
		};
	});
	source.connect(processor).connect(context.destination);
	const quantumStart =
		(Math.ceil(((context.currentTime + 0.05) * SAMPLE_RATE) / 128) * 128) /
		SAMPLE_RATE;
	source.start(quantumStart);
	await captured;
	await context.close();
	const output = Array.from({ length: CHANNELS }, () => new Float32Array(frames));
	let offset = 0;
	for (const chunk of chunks) {
		const length = Math.min(chunk[0].length, frames - offset);
		for (let channel = 0; channel < CHANNELS; channel += 1) {
			output[channel].set(chunk[channel].subarray(0, length), offset);
		}
		offset += length;
		if (offset >= frames) break;
	}
	return output;
}

function compare(actual, reference) {
	let residualSquared = 0;
	let signalSquared = 0;
	let maxAbsolute = 0;
	let maxLocation = null;
	let samples = 0;
	for (let channel = 0; channel < CHANNELS; channel += 1) {
		for (let frame = 0; frame < actual[channel].length; frame += 1) {
			const residual = actual[channel][frame] - reference[channel][frame];
			residualSquared += residual * residual;
			signalSquared += reference[channel][frame] * reference[channel][frame];
			if (Math.abs(residual) > maxAbsolute) {
				maxAbsolute = Math.abs(residual);
				maxLocation = {
					channel,
					frame,
					actual: actual[channel][frame],
					reference: reference[channel][frame],
				};
			}
			samples += 1;
		}
	}
	const residualRms = Math.sqrt(residualSquared / samples);
	const signalRms = Math.sqrt(signalSquared / samples);
	const channelRelativeResidualDb = actual.map((channel, channelIndex) => {
		let channelResidualSquared = 0;
		let channelSignalSquared = 0;
		for (let frame = 0; frame < channel.length; frame += 1) {
			const residual = channel[frame] - reference[channelIndex][frame];
			channelResidualSquared += residual * residual;
			channelSignalSquared +=
				reference[channelIndex][frame] * reference[channelIndex][frame];
		}
		return (
			20 *
			Math.log10(
				Math.sqrt(channelResidualSquared / channelSignalSquared) || Number.MIN_VALUE,
			)
		);
	});
	return {
		residualDbfs: 20 * Math.log10(residualRms || Number.MIN_VALUE),
		relativeResidualDb:
			20 * Math.log10(residualRms / signalRms || Number.MIN_VALUE),
		maxAbsolute,
		maxLocation,
		channelRelativeResidualDb,
	};
}

function impulseEvents(channels, threshold = 1e-6, limit = 24) {
	const events = [];
	for (let channel = 0; channel < channels.length; channel += 1) {
		for (let frame = 0; frame < channels[channel].length; frame += 1) {
			const sample = channels[channel][frame];
			if (Math.abs(sample) > threshold) {
				events.push({ channel, frame, sample });
				if (events.length >= limit) return events;
			}
		}
	}
	return events;
}

export async function runParity() {
	const measurements = [];
	const sources = fixtures();
	for (const [fixtureName, input] of Object.entries(sources)) {
		for (const [effectName, effects] of Object.entries(EFFECT_CASES)) {
			const wasm = await renderWasm(input, effects);
			const tone = await renderTone(input, effects);
			const comparison = compare(tone, wasm);
			if (comparison.maxLocation) {
				comparison.maxLocation.input =
					input[comparison.maxLocation.channel][comparison.maxLocation.frame];
			}
			if (
				fixtureName === "impulse" &&
				["feedbackDelay", "chorus"].includes(effectName)
			) {
				comparison.toneEvents = impulseEvents(tone);
				comparison.wasmEvents = impulseEvents(wasm);
			}
			measurements.push({
				fixture: fixtureName,
				effect: effectName,
				comparison: "tone-vs-wasm",
				...comparison,
			});
		}
	}

	let worklet;
	try {
		const input = sources.mixed.map((channel) => channel.slice(0, 8_192));
		const workletCases = {
			basic: EFFECT_CASES.combined,
			distortion: EFFECT_CASES.distortion,
			delay: EFFECT_CASES.feedbackDelay,
			compressor: EFFECT_CASES.compressor,
			chorus: EFFECT_CASES.chorus,
			reverb: {
				reverbEnabled: true,
				reverbDecaySeconds: 1.5,
				reverbPreDelaySeconds: 0.01,
				reverbWet: 0.35,
			},
			all: {
				...EFFECT_CASES.combinedAll,
				...EFFECT_CASES.compressor,
				...EFFECT_CASES.chorus,
				reverbEnabled: true,
				reverbDecaySeconds: 1.5,
				reverbPreDelaySeconds: 0.01,
				reverbWet: 0.35,
			},
		};
		worklet = { status: "ok", cases: {} };
		for (const [name, effects] of Object.entries(workletCases)) {
			const workletOutput = await renderWorklet(input, effects);
			worklet.cases[name] = {
				vsWasm: compare(workletOutput, await renderWasm(input, effects)),
				vsInput: compare(workletOutput, input),
			};
		}
	} catch (error) {
		worklet = { status: "unavailable", error: String(error) };
	}
	const result = {
		userAgent: navigator.userAgent,
		sampleRate: SAMPLE_RATE,
		measurements,
		worklet,
	};
	document.querySelector("#output").textContent = JSON.stringify(result, null, 2);
	return result;
}

window.runParity = runParity;
