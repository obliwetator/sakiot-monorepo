import { beforeAll, describe, expect, test } from "bun:test";
import { ClipEditorEngine, type EditorAudioGraphFactory } from "./engine";
import { type ClipEdit, DEFAULT_EFFECTS, type TimelineSegment } from "./model";
import { warmSharedDsp } from "./sharedDsp";

beforeAll(async () => {
	await warmSharedDsp();
});

Object.defineProperty(globalThis, "window", {
	value: globalThis,
	configurable: true,
});

class MockSource {
	started = false;
	stopped = false;
	detune = { value: 0 };
	playbackRate = { value: 1 };
	buffer: unknown = null;
	start() {
		this.started = true;
	}
	stop() {
		this.stopped = true;
	}
	connect() {}
}

class MockFilter {
	type = "";
	frequency = { value: 0 };
	gain = { value: 0 };
	connect() {}
}

class MockGain {
	gain = { value: 0 };
	connect() {}
}

class MockAudioContext {
	state = "running" as const;
	currentTime = 0;
	destination: Record<string, never> = {};
	sources: MockSource[] = [];
	createBufferSource() {
		const source = new MockSource();
		this.sources.push(source);
		return source as unknown as AudioBufferSourceNode;
	}
	createGain() {
		return new MockGain() as unknown as GainNode;
	}
	createBiquadFilter() {
		return new MockFilter() as unknown as BiquadFilterNode;
	}
	resume() {}
	close() {}
}

globalThis.AudioContext = MockAudioContext as unknown as typeof AudioContext;

const createMockAudioGraph: EditorAudioGraphFactory = () => ({
	createSegment: () => ({
		connectSource() {},
		setEffects() {},
		dispose() {},
	}),
	setMasterVolume() {},
	dispose() {},
});

const segment: TimelineSegment = {
	id: "seg-1",
	track: 0,
	source: "clip",
	sourceId: "clip-1",
	sourceIn: 0,
	sourceOut: 60,
	timelineStart: 0,
	effects: { ...DEFAULT_EFFECTS },
};

const edit: ClipEdit = {
	segments: [segment],
	tracks: 1,
	masterVolumeDb: 0,
};

const buffers = new Map<string, AudioBuffer>([
	["clip-1", { duration: 60 } as unknown as AudioBuffer],
]);

describe("ClipEditorEngine playback", () => {
	test("pause stops the sources and keeps the playhead for resume", async () => {
		const engine = new ClipEditorEngine(createMockAudioGraph);
		engine.play(edit, 0, buffers, false);
		expect(engine.isPlaying).toBe(true);
		await Bun.sleep(30);

		engine.pause();
		expect(engine.isPlaying).toBe(false);
		const pausedAt = engine.positionSec;
		expect(pausedAt).toBeGreaterThan(0);
		await Bun.sleep(20);
		expect(engine.positionSec).toBeCloseTo(pausedAt, 2);

		engine.play(edit, engine.positionSec, buffers, false);
		expect(engine.isPlaying).toBe(true);
		expect(engine.positionSec).toBeCloseTo(pausedAt, 2);
	});

	test("pause cancels the scheduled end so it cannot restart", async () => {
		const engine = new ClipEditorEngine(createMockAudioGraph);
		engine.play(edit, 0, buffers, true);
		expect(engine.isPlaying).toBe(true);
		await Bun.sleep(5);
		engine.pause();
		await Bun.sleep(30);
		expect(engine.isPlaying).toBe(false);
	});

	test("reversed segments schedule a negative playback rate", () => {
		const contexts: MockAudioContext[] = [];
		globalThis.AudioContext = class extends MockAudioContext {
			constructor() {
				super();
				contexts.push(this);
			}
		} as unknown as typeof AudioContext;
		const reversed: TimelineSegment = {
			...segment,
			effects: { ...segment.effects, reverse: true },
		};
		const engine = new ClipEditorEngine(createMockAudioGraph);
		engine.play({ ...edit, segments: [reversed] }, 0, buffers, false);
		const source = contexts[0]?.sources[0];
		expect(source?.playbackRate.value).toBe(-1);
	});
});
