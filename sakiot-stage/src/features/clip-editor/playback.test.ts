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
	sampleRate = 48_000;
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
	createBuffer(channels: number, length: number, sampleRate: number) {
		return new TestAudioBuffer(
			channels,
			length,
			sampleRate,
		) as unknown as AudioBuffer;
	}
	resume() {}
	close() {}
}

class TestAudioBuffer {
	readonly data: Float32Array[];
	duration: number;

	constructor(
		readonly numberOfChannels: number,
		readonly length: number,
		readonly sampleRate: number,
	) {
		this.duration = length / sampleRate;
		this.data = Array.from(
			{ length: numberOfChannels },
			() => new Float32Array(length),
		);
	}

	getChannelData(channel: number) {
		return this.data[channel] ?? new Float32Array();
	}
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
	mutedTracks: [false],
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

	test("loops at the longest non-muted segment", async () => {
		const contexts: MockAudioContext[] = [];
		const previousAudioContext = globalThis.AudioContext;
		globalThis.AudioContext = class extends MockAudioContext {
			constructor() {
				super();
				contexts.push(this);
			}
		} as unknown as typeof AudioContext;
		const audible = {
			...segment,
			id: "audible",
			sourceOut: 0.05,
		};
		const muted = {
			...segment,
			id: "muted",
			sourceId: "muted-clip",
			sourceOut: 0.5,
			track: 1,
		};
		const loopEdit: ClipEdit = {
			segments: [audible, muted],
			tracks: 2,
			mutedTracks: [false, true],
			masterVolumeDb: 0,
		};
		const engine = new ClipEditorEngine(createMockAudioGraph);
		try {
			engine.play(
				loopEdit,
				0,
				new Map([["clip-1", { duration: 0.05 } as unknown as AudioBuffer]]),
				true,
			);
			await Bun.sleep(120);
			expect(contexts[0]?.sources.length ?? 0).toBeGreaterThan(1);
		} finally {
			engine.dispose();
			globalThis.AudioContext = previousAudioContext;
		}
	});

	test("reversed segments schedule a negative playback rate", async () => {
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
		await Bun.sleep(0);
		const source = contexts[0]?.sources[0];
		expect(source?.playbackRate.value).toBe(-1);
	});

	test("uses cached preprocessing and applies streaming edits live", async () => {
		let processing = "";
		let updates = 0;
		const streamingGraph: EditorAudioGraphFactory = () => ({
			prepare: async () => true,
			createSegment(_effects, nextProcessing) {
				processing = nextProcessing;
				return {
					connectSource() {},
					setEffects() {
						updates += 1;
					},
					dispose() {},
				};
			},
			setMasterVolume() {},
			dispose() {},
		});
		const source = new TestAudioBuffer(2, 4_800, 48_000);
		const streamingSegment: TimelineSegment = {
			...segment,
			sourceOut: 0.1,
			effects: { ...segment.effects, pitchCents: 200 },
		};
		const engine = new ClipEditorEngine(streamingGraph);
		engine.play(
			{ ...edit, segments: [streamingSegment] },
			0,
			new Map([["clip-1", source as unknown as AudioBuffer]]),
			false,
		);
		for (let attempt = 0; attempt < 20 && !processing; attempt += 1) {
			await Bun.sleep(25);
		}

		expect(processing).toBe("streaming");
		engine.applySegmentEffects(streamingSegment.id, {
			...streamingSegment.effects,
			volumeDb: -3,
		});
		expect(updates).toBe(1);
		engine.dispose();
	});
});
