import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import init, { WasmSegmentProcessor } from "../pkg/sakiot_dsp.js";

const dspRoot = fileURLToPath(new URL("..", import.meta.url));
const wasmBytes = await readFile(new URL("../pkg/sakiot_dsp_bg.wasm", import.meta.url));
await init({ module_or_path: wasmBytes });

const channels = 2;
const frames = 4_097;
const input = new Float32Array(frames * channels);
let noise = 0x12345678;
for (let frame = 0; frame < frames; frame += 1) {
	noise ^= noise << 13;
	noise ^= noise >>> 17;
	noise ^= noise << 5;
	noise >>>= 0;
	const random = (noise / 0xffffffff) * 2 - 1;
	for (let channel = 0; channel < channels; channel += 1) {
		const frequency = 173 + channel * 619;
		input[frame * channels + channel] =
			Math.sin((Math.PI * 2 * frequency * frame) / 48000) * 0.2 +
			random * 0.05;
	}
}

const wasm = new WasmSegmentProcessor(48000, channels);
const defaultEffects = {
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
};
const config = (effects) => ({
	version: 2,
	effects: { ...defaultEffects, ...effects },
});
if (wasm.set_effect_config({ ...config({}), version: 3 })) {
	throw new Error("WASM accepted an unknown effect configuration version");
}
wasm.set_effect_config(config({
	volumeDb: -3,
	bassDb: 5,
	midDb: -4,
	trebleDb: 2.5,
	distortionAmount: 0.7,
	distortionWet: 1,
	delaySeconds: 0.125,
	delayFeedback: 0.4,
	delayWet: 1,
	compressorEnabled: true,
	chorusEnabled: true,
	reverbEnabled: true,
	reverbDecaySeconds: 0.05,
	reverbPreDelaySeconds: 0.002,
	reverbWet: 0.35,
}));
const wasmOutput = input.slice();
wasm.process_interleaved(wasmOutput);

const native = spawn(
	"cargo",
	[
		"run",
		"--offline",
		"--quiet",
		"--example",
		"process_raw",
		"--",
		"48000",
		String(channels),
		"-3",
		"5",
		"-4",
		"2.5",
		"0.7",
		"1",
		"0.125",
		"0.4",
		"1",
		"true",
		"-24",
		"30",
		"12",
		"0.003",
		"0.25",
		"true",
		"1.5",
		"3.5",
		"0.7",
		"180",
		"0",
		"0.5",
		"true",
		"0.05",
		"0.002",
		"0.35",
		String(0x53414b49),
	],
	{
		cwd: dspRoot,
	},
);
const stdout = [];
const stderr = [];
native.stdout.on("data", (chunk) => stdout.push(chunk));
native.stderr.on("data", (chunk) => stderr.push(chunk));
native.stdin.end(Buffer.from(input.buffer, input.byteOffset, input.byteLength));
const status = await new Promise((resolve, reject) => {
	native.once("error", reject);
	native.once("close", resolve);
});
if (status !== 0) {
	throw new Error(`native fixture failed: ${Buffer.concat(stderr).toString()}`);
}
const nativeBytes = Buffer.concat(stdout);
if (nativeBytes.byteLength !== wasmOutput.byteLength) {
	throw new Error(
		`native returned ${nativeBytes.byteLength} bytes; expected ${wasmOutput.byteLength}`,
	);
}
const nativeOutput = new Float32Array(
	nativeBytes.buffer,
	nativeBytes.byteOffset,
	nativeBytes.byteLength / Float32Array.BYTES_PER_ELEMENT,
);

let residualSquared = 0;
let signalSquared = 0;
let maxAbsolute = 0;
for (let index = 0; index < wasmOutput.length; index += 1) {
	const residual = wasmOutput[index] - nativeOutput[index];
	residualSquared += residual * residual;
	signalSquared += nativeOutput[index] * nativeOutput[index];
	maxAbsolute = Math.max(maxAbsolute, Math.abs(residual));
}
const relativeResidualDb =
	20 * Math.log10(Math.sqrt(residualSquared / signalSquared) || Number.MIN_VALUE);
console.log(
	`WASM/native parity: relative=${relativeResidualDb.toFixed(2)} dB, max_abs=${maxAbsolute.toExponential(3)}`,
);
if (relativeResidualDb >= -90) {
	throw new Error("WASM/native residual is above the prototype threshold");
}

const smoothingProbe = new WasmSegmentProcessor(48000, 1);
const smoothingPrefix = new Float32Array(16).fill(1);
smoothingProbe.process_interleaved(smoothingPrefix);
smoothingProbe.set_effect_config(config({ volumeDb: -60 }));
const smoothingOutput = new Float32Array(242).fill(1);
smoothingProbe.process_interleaved(smoothingOutput);
smoothingProbe.free();
if (smoothingOutput[0] !== smoothingPrefix.at(-1)) {
	throw new Error("WASM parameter update was not output-continuous");
}
if (Math.abs(smoothingOutput[240] - 0.001) > 1e-7) {
	throw new Error("WASM parameter smoothing did not settle after 5 ms");
}

wasm.set_effect_config(
	config({ pitchCents: 700, rate: 1.35, reverse: true, tailSeconds: 0.01 }),
);
const offlineWasm = wasm.render_clip_interleaved(input);
const offlineNative = spawn(
	"cargo",
	[
		"run",
		"--offline",
		"--quiet",
		"--example",
		"render_clip_raw",
		"--",
		"48000",
		String(channels),
		"700",
		"1.35",
		"true",
		"0.01",
	],
	{ cwd: dspRoot },
);
const offlineStdout = [];
const offlineStderr = [];
offlineNative.stdout.on("data", (chunk) => offlineStdout.push(chunk));
offlineNative.stderr.on("data", (chunk) => offlineStderr.push(chunk));
offlineNative.stdin.end(Buffer.from(input.buffer, input.byteOffset, input.byteLength));
const offlineStatus = await new Promise((resolve, reject) => {
	offlineNative.once("error", reject);
	offlineNative.once("close", resolve);
});
if (offlineStatus !== 0) {
	throw new Error(
		`native offline fixture failed: ${Buffer.concat(offlineStderr).toString()}`,
	);
}
const offlineBytes = Buffer.concat(offlineStdout);
const offlineOutput = new Float32Array(
	offlineBytes.buffer,
	offlineBytes.byteOffset,
	offlineBytes.byteLength / Float32Array.BYTES_PER_ELEMENT,
);
if (offlineOutput.length !== offlineWasm.length) {
	throw new Error(
		`offline length mismatch: native=${offlineOutput.length}, wasm=${offlineWasm.length}`,
	);
}
let offlineResidualSquared = 0;
let offlineSignalSquared = 0;
let offlineMaxAbsolute = 0;
for (let index = 0; index < offlineWasm.length; index += 1) {
	const residual = offlineWasm[index] - offlineOutput[index];
	offlineResidualSquared += residual * residual;
	offlineSignalSquared += offlineOutput[index] * offlineOutput[index];
	offlineMaxAbsolute = Math.max(offlineMaxAbsolute, Math.abs(residual));
}
const offlineResidualDb =
	20 *
	Math.log10(
		Math.sqrt(offlineResidualSquared / offlineSignalSquared) || Number.MIN_VALUE,
	);
console.log(
	`Offline rate/pitch/tail parity: relative=${offlineResidualDb.toFixed(2)} dB, max_abs=${offlineMaxAbsolute.toExponential(3)}, frames=${offlineWasm.length / channels}`,
);
if (offlineResidualDb >= -90) {
	throw new Error("offline native/WASM residual is above the prototype threshold");
}
