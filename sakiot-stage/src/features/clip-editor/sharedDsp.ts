import type { TimelineSegment } from "./model";
import { SHARED_DSP_EFFECT_CONFIG_VERSION } from "./sharedDspConfig";

export {
	SHARED_DSP_EFFECT_CONFIG_VERSION,
	sharedDspEffectConfig,
} from "./sharedDspConfig";

import type {
	SharedDspPcm,
	SharedDspRender,
	SharedDspWorkerDspRequest,
	SharedDspWorkerResponse,
} from "./sharedDspProtocol";

export type { SharedDspPcm, SharedDspRender } from "./sharedDspProtocol";

interface CachedValue<T> {
	value: T | null;
	promise: Promise<T | null> | null;
	audioBuffer: AudioBuffer | null;
	lanes: Set<string>;
}

type WorkerOperation = SharedDspWorkerDspRequest extends infer Request
	? Request extends SharedDspWorkerDspRequest
		? Omit<Request, "id">
		: never
	: never;
type WorkerResult = SharedDspPcm | SharedDspRender;

interface QueuedOperation {
	lane: string;
	request: WorkerOperation;
	resolve(result: WorkerResult | null): void;
}

const renderCache = new WeakMap<
	AudioBuffer,
	Map<string, CachedValue<SharedDspRender>>
>();
const preprocessCache = new WeakMap<
	AudioBuffer,
	Map<string, CachedValue<SharedDspPcm>>
>();
const pendingWorkerRequests = new Map<
	number,
	{
		resolve(result: WorkerResult): void;
		reject(error: Error): void;
	}
>();
const queuedPreprocessByLane = new Map<string, QueuedOperation>();
const queuedProcessByLane = new Map<string, QueuedOperation>();

let worker: Worker | null = null;
let initialization: Promise<void> | null = null;
let resolveInitialization: (() => void) | null = null;
let ready = false;
let failed = false;
let nextRequestId = 0;
let activeOperation: QueuedOperation | null = null;

/** Begin loading the worker-owned shared module without delaying the editor. */
export function warmSharedDsp(): Promise<void> {
	if (initialization) return initialization;
	initialization = new Promise<void>((resolve) => {
		resolveInitialization = resolve;
		if (typeof Worker === "undefined") {
			failed = true;
			resolveInitialization = null;
			resolve();
			return;
		}

		try {
			worker = new Worker(new URL("./sharedDsp.worker.ts", import.meta.url), {
				type: "module",
			});
			worker.onmessage = handleWorkerMessage;
			worker.onerror = () => failWorker("Shared DSP worker crashed");
			worker.postMessage({ type: "initialize" });
		} catch {
			failWorker("Shared DSP worker could not be started");
		}
	});
	return initialization;
}

export function sharedDspAvailable(): boolean {
	return ready && !failed;
}

/** True after initialization either succeeds or irrecoverably fails. */
export function sharedDspSettled(): boolean {
	return ready || failed;
}

/**
 * Render and cache reverse/pitch/rate/tail independently from live effects.
 * Geometry work has priority over queued waveform work so playback can start
 * as soon as its reusable prefix is ready.
 */
export function requestSharedSegmentPreprocessedPcm(
	source: AudioBuffer,
	segment: TimelineSegment,
): Promise<SharedDspPcm | null> {
	if (typeof source.getChannelData !== "function") return Promise.resolve(null);
	const key = sharedDspPreprocessKey(segment);
	const sourceCache =
		preprocessCache.get(source) ?? new Map<string, CachedValue<SharedDspPcm>>();
	preprocessCache.set(source, sourceCache);
	releaseStaleLaneEntries(sourceCache, segment.id, key);
	const existing = sourceCache.get(key);
	if (existing) {
		existing.lanes.add(segment.id);
		if (existing.value) return Promise.resolve(existing.value);
		if (existing.promise) return existing.promise;
	}

	const startFrame = Math.max(
		0,
		Math.min(source.length, Math.round(segment.sourceIn * source.sampleRate)),
	);
	const endFrame = Math.max(
		startFrame,
		Math.min(source.length, Math.round(segment.sourceOut * source.sampleRate)),
	);
	if (endFrame === startFrame) return Promise.resolve(null);

	const entry: CachedValue<SharedDspPcm> = {
		value: null,
		promise: null,
		audioBuffer: null,
		lanes: new Set([segment.id]),
	};
	const promise = enqueueOperation(`preprocess:${segment.id}`, {
		type: "preprocess",
		sampleRate: source.sampleRate,
		left: source.getChannelData(0).slice(startFrame, endFrame),
		right: source
			.getChannelData(Math.min(1, source.numberOfChannels - 1))
			.slice(startFrame, endFrame),
		effects: { ...segment.effects },
	}).then((result) => {
		entry.promise = null;
		const pcm = isPcm(result) ? result : null;
		if (pcm) entry.value = pcm;
		else sourceCache.delete(key);
		return pcm;
	});
	entry.promise = promise;
	sourceCache.set(key, entry);
	return promise;
}

/**
 * Render the exact complete segment for waveform display. It reuses the
 * geometry cache, then runs only the streaming suffix in the worker.
 */
export function requestSharedSegmentRender(
	source: AudioBuffer,
	segment: TimelineSegment,
): Promise<SharedDspRender | null> {
	if (typeof source.getChannelData !== "function") return Promise.resolve(null);
	const key = sharedDspRenderKey(segment);
	const sourceCache =
		renderCache.get(source) ?? new Map<string, CachedValue<SharedDspRender>>();
	renderCache.set(source, sourceCache);
	releaseStaleLaneEntries(sourceCache, segment.id, key);
	const existing = sourceCache.get(key);
	if (existing) {
		existing.lanes.add(segment.id);
		if (existing.value) return Promise.resolve(existing.value);
		if (existing.promise) return existing.promise;
	}

	const entry: CachedValue<SharedDspRender> = {
		value: null,
		promise: null,
		audioBuffer: null,
		lanes: new Set([segment.id]),
	};
	const promise = requestSharedSegmentPreprocessedPcm(source, segment)
		.then((pcm) => {
			if (!pcm) return null;
			return enqueueOperation(`process:${segment.id}`, {
				type: "process",
				pcm: { ...pcm, interleaved: pcm.interleaved.slice() },
				effects: { ...segment.effects },
			});
		})
		.then((result) => {
			entry.promise = null;
			const render = isRender(result) ? result : null;
			if (render) entry.value = render;
			else sourceCache.delete(key);
			return render;
		});
	entry.promise = promise;
	sourceCache.set(key, entry);
	return promise;
}

function releaseStaleLaneEntries<T>(
	sourceCache: Map<string, CachedValue<T>>,
	lane: string,
	requestedKey: string,
) {
	for (const [key, entry] of sourceCache) {
		if (key === requestedKey || !entry.lanes.delete(lane)) continue;
		if (entry.lanes.size === 0) sourceCache.delete(key);
	}
}

/** The complete effect-processed PCM used for exact waveform generation. */
export async function requestSharedSegmentPcm(
	source: AudioBuffer,
	segment: TimelineSegment,
): Promise<SharedDspPcm | null> {
	return (await requestSharedSegmentRender(source, segment))?.pcm ?? null;
}

/** Build a Web Audio buffer from either cached geometry or a complete render. */
export async function requestSharedSegment(
	context: AudioContext,
	source: AudioBuffer,
	segment: TimelineSegment,
	preprocessOnly = false,
): Promise<AudioBuffer | null> {
	if (typeof context.createBuffer !== "function") return null;
	const pcm = preprocessOnly
		? await requestSharedSegmentPreprocessedPcm(source, segment)
		: await requestSharedSegmentPcm(source, segment);
	if (!pcm) return null;

	const sourceCache = preprocessOnly
		? preprocessCache.get(source)
		: renderCache.get(source);
	const key = preprocessOnly
		? sharedDspPreprocessKey(segment)
		: sharedDspRenderKey(segment);
	const entry = sourceCache?.get(key);
	if (entry?.audioBuffer) return entry.audioBuffer;
	const output = context.createBuffer(pcm.channels, pcm.frames, pcm.sampleRate);
	for (let channel = 0; channel < pcm.channels; channel += 1) {
		const channelData = output.getChannelData(channel);
		for (let frame = 0; frame < pcm.frames; frame += 1) {
			channelData[frame] = pcm.interleaved[frame * pcm.channels + channel] ?? 0;
		}
	}
	if (entry) entry.audioBuffer = output;
	return output;
}

function enqueueOperation(
	lane: string,
	request: WorkerOperation,
): Promise<WorkerResult | null> {
	return new Promise((resolve) => {
		const queue =
			request.type === "preprocess"
				? queuedPreprocessByLane
				: queuedProcessByLane;
		const superseded = queue.get(lane);
		if (superseded) superseded.resolve(null);
		queue.set(lane, { lane, request, resolve });
		pumpOperationQueue();
	});
}

function pumpOperationQueue() {
	if (activeOperation) return;
	const next =
		queuedPreprocessByLane.values().next().value ??
		queuedProcessByLane.values().next().value;
	if (!next) return;
	const operation = next as QueuedOperation;
	queuedPreprocessByLane.delete(operation.lane);
	queuedProcessByLane.delete(operation.lane);
	activeOperation = operation;
	void sendWorkerOperation(operation.request)
		.then(operation.resolve)
		.catch(() => operation.resolve(null))
		.finally(() => {
			activeOperation = null;
			pumpOperationQueue();
		});
}

async function sendWorkerOperation(
	request: WorkerOperation,
): Promise<WorkerResult> {
	await warmSharedDsp();
	if (!worker || !sharedDspAvailable()) {
		throw new Error("Shared DSP worker is unavailable");
	}
	const id = ++nextRequestId;
	const response = new Promise<WorkerResult>((resolve, reject) => {
		pendingWorkerRequests.set(id, { resolve, reject });
	});
	if (request.type === "preprocess") {
		worker.postMessage({ ...request, id }, [
			request.left.buffer,
			request.right.buffer,
		]);
	} else {
		worker.postMessage({ ...request, id }, [request.pcm.interleaved.buffer]);
	}
	return response;
}

function handleWorkerMessage(event: MessageEvent<SharedDspWorkerResponse>) {
	const message = event.data;
	if (message.type === "ready") {
		ready = true;
		resolveInitialization?.();
		resolveInitialization = null;
		return;
	}
	if (message.type === "preprocessed" || message.type === "rendered") {
		const pending = pendingWorkerRequests.get(message.id);
		pendingWorkerRequests.delete(message.id);
		pending?.resolve(
			message.type === "preprocessed" ? message.pcm : message.render,
		);
		return;
	}
	if (message.id !== undefined) {
		const pending = pendingWorkerRequests.get(message.id);
		pendingWorkerRequests.delete(message.id);
		pending?.reject(new Error(message.message));
		return;
	}
	failWorker(message.message);
}

function failWorker(message: string) {
	failed = true;
	ready = false;
	worker?.terminate();
	worker = null;
	for (const pending of pendingWorkerRequests.values()) {
		pending.reject(new Error(message));
	}
	pendingWorkerRequests.clear();
	resolveInitialization?.();
	resolveInitialization = null;
}

function isPcm(value: WorkerResult | null): value is SharedDspPcm {
	return value !== null && "interleaved" in value;
}

function isRender(value: WorkerResult | null): value is SharedDspRender {
	return value !== null && "pcm" in value;
}

/** Cache identity for the length-changing prefix only. */
export function sharedDspPreprocessKey(segment: TimelineSegment): string {
	const effect = segment.effects;
	return [
		SHARED_DSP_EFFECT_CONFIG_VERSION,
		segment.sourceIn,
		segment.sourceOut,
		effect.pitchCents,
		effect.rate,
		effect.tailSeconds,
		effect.reverse ? 1 : 0,
	].join(":");
}

export function sharedDspRenderKey(segment: TimelineSegment): string {
	const effect = segment.effects;
	return [
		sharedDspPreprocessKey(segment),
		effect.volumeDb,
		effect.bassDb,
		effect.midDb,
		effect.trebleDb,
		effect.distortionAmount,
		effect.distortionWet,
		effect.delaySeconds,
		effect.delayFeedback,
		effect.delayWet,
		effect.compressorEnabled ? 1 : 0,
		effect.compressorThresholdDb,
		effect.compressorKneeDb,
		effect.compressorRatio,
		effect.compressorAttackSeconds,
		effect.compressorReleaseSeconds,
		effect.chorusEnabled ? 1 : 0,
		effect.chorusFrequencyHz,
		effect.chorusDelayMs,
		effect.chorusDepth,
		effect.chorusSpreadDegrees,
		effect.chorusFeedback,
		effect.chorusWet,
		effect.reverbEnabled ? 1 : 0,
		effect.reverbDecaySeconds,
		effect.reverbPreDelaySeconds,
		effect.reverbWet,
		effect.reverbSeed,
	].join(":");
}
