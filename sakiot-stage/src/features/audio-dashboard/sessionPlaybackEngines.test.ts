import { afterEach, describe, expect, spyOn, test } from "bun:test";
import type { PlaybackSegment } from "./logicalSessionTimeline";
import { SegmentedSessionEngine } from "./segmentedSessionEngine";
import { SilenceFreeEngine } from "./silenceFreeEngine";

// Real media playback needs a browser and is exercised by the Playwright
// suites; here a scripted element and frame clock drive the engine.

function segmented(segments: PlaybackSegment[] = [], durationMs = 10_000) {
	const errors: (string | null)[] = [];
	const engine = new SegmentedSessionEngine({
		segments,
		durationMs,
		volume: 1,
		playbackRate: 1,
		onError: (message) => errors.push(message),
		onLoopDisabled: () => {},
	});
	return { engine, errors };
}

function silenceFree() {
	return new SilenceFreeEngine({
		mediaUrl: "https://sakiot.test/mix.ogg",
		initialDurationMs: 8_000,
		volume: 1,
		playbackRate: 1,
		onLoopDisabled: () => {},
	});
}

describe("SegmentedSessionEngine", () => {
	test("a paused start moves the playhead, clamped to the session", () => {
		const { engine } = segmented();
		engine.setSeekPreviewMs(4_000);
		engine.startAt(25_000, false);
		expect(engine.getSnapshot()).toMatchObject({
			positionMs: 10_000,
			seekPreviewMs: null,
			playing: false,
		});
	});

	test("a shorter session clamps the playhead and the seek preview", () => {
		const { engine } = segmented();
		engine.startAt(9_000, false);
		engine.setSeekPreviewMs(9_500);
		engine.setTimeline([], 6_000);
		expect(engine.getSnapshot()).toMatchObject({
			positionMs: 6_000,
			seekPreviewMs: 6_000,
		});
	});

	test("playing where no segment exists stays stopped and drops the bound", () => {
		const { engine } = segmented();
		engine.togglePreview([1_000, 2_000], false);
		expect(engine.getSnapshot()).toMatchObject({
			positionMs: 1_000,
			playing: false,
			boundActive: false,
		});
	});

	test("dispose leaves the engine usable, as Strict Mode remounts need", () => {
		const { engine } = segmented();
		engine.dispose();
		engine.startAt(3_000, false);
		expect(engine.getSnapshot().positionMs).toBe(3_000);
	});
});

/** An audio element whose clock and events the test drives. */
class FakeAudio extends EventTarget {
	static created: FakeAudio[] = [];
	src = "";
	currentTime = 0;
	duration = Number.NaN;
	volume = 1;
	playbackRate = 1;
	crossOrigin: string | null = null;
	preload = "";
	constructor() {
		super();
		FakeAudio.created.push(this);
	}
	canPlayType(type: string) {
		// The native-HLS path, which needs no hls.js.
		return type === "application/vnd.apple.mpegurl" ? "probably" : "";
	}
	play() {
		this.dispatchEvent(new Event("playing"));
		return Promise.resolve();
	}
	pause() {}
	load() {}
	removeAttribute(name: string) {
		if (name === "src") this.src = "";
	}
	/** Plays to `seconds` and reports it, as a frame or timeupdate would. */
	advance(seconds: number) {
		this.currentTime = seconds;
		this.dispatchEvent(new Event("timeupdate"));
	}
}

let frames: FrameRequestCallback[] = [];
let clock: { mockRestore: () => void } | null = null;
const globals = globalThis as Record<string, unknown>;
const savedWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
const saved = {
	Audio: globals.Audio,
	requestAnimationFrame: globals.requestAnimationFrame,
	cancelAnimationFrame: globals.cancelAnimationFrame,
};
function fakeBrowser() {
	FakeAudio.created = [];
	frames = [];
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: { location: { origin: "https://sakiot.test" } },
	});
	globals.Audio = FakeAudio;
	// Silence advances on the wall clock from the frame times given below.
	clock = spyOn(performance, "now").mockReturnValue(0);
	globals.requestAnimationFrame = (callback: FrameRequestCallback) =>
		frames.push(callback);
	globals.cancelAnimationFrame = () => {};
}
/** Runs the pending animation frames at wall-clock `now`. */
function frame(now: number) {
	const pending = frames;
	frames = [];
	for (const callback of pending) callback(now);
}
afterEach(() => {
	Object.assign(globals, saved);
	if (savedWindow) Object.defineProperty(globalThis, "window", savedWindow);
	else delete globals.window;
	clock?.mockRestore();
	clock = null;
});

const liveFragment = (end_ms: number): PlaybackSegment => ({
	kind: "active_hls",
	start_ms: 0,
	end_ms,
	audio_file_id: "fragment-1",
	media_url: "/api/audio/sessions/1/segments/fragment-1",
	hls_playlist_url: "/api/audio/sessions/1/live/fragment-1/playlist.m3u8",
});

describe("SegmentedSessionEngine while the session records", () => {
	test("a live fragment plays past the end known when it started", () => {
		fakeBrowser();
		const { engine } = segmented([liveFragment(5_000)], 5_000);
		engine.startAt(1_000, true);
		const audio = FakeAudio.created[0];
		if (!audio) throw new Error("no media element");
		audio.dispatchEvent(new Event("loadedmetadata"));
		expect(audio.currentTime).toBe(1);

		// The manifest said 5 s; the live edge has moved on since.
		audio.advance(6);
		expect(engine.getSnapshot()).toMatchObject({
			positionMs: 6_000,
			playing: true,
		});
		// A revision that still trails the live edge does not pull it back.
		engine.setTimeline([liveFragment(5_500)], 5_500);
		expect(engine.getSnapshot().positionMs).toBe(6_000);
	});

	test("seeking near the end of a live fragment stays behind its live edge", () => {
		fakeBrowser();
		const { engine } = segmented([liveFragment(15_000)], 15_000);
		engine.startAt(1_000, true);
		const audio = FakeAudio.created[0];
		if (!audio) throw new Error("no media element");
		// hls.js reports the playlist's end as the element's duration; the
		// manifest's end ("now") is further on.
		audio.duration = 10;
		audio.dispatchEvent(new Event("loadedmetadata"));
		engine.seek(14_500, [0, 15_000], false);
		expect(audio.currentTime).toBe(8);
		expect(engine.getSnapshot().positionMs).toBe(8_000);
		engine.seek(5_000, [0, 15_000], false);
		expect(audio.currentTime).toBe(5);
		expect(engine.getSnapshot().positionMs).toBe(5_000);
	});

	test("seeking to the very end of a recording session plays from its live edge", () => {
		fakeBrowser();
		const { engine } = segmented([liveFragment(15_000)], 15_000);
		// Paused: the playhead parks at the end without leaving the fragment,
		// so play resumes there instead of restarting from the beginning.
		engine.seek(15_000, [0, 15_000], false);
		expect(engine.getSnapshot()).toMatchObject({
			positionMs: 14_999,
			playing: false,
		});
		engine.togglePlay([0, 15_000], false);
		const audio = FakeAudio.created[0];
		if (!audio) throw new Error("no media element");
		audio.duration = 10;
		audio.dispatchEvent(new Event("loadedmetadata"));
		expect(engine.getSnapshot()).toMatchObject({
			positionMs: 8_000,
			playing: true,
		});
		// Playing: the same seek stays in the live media.
		engine.seek(15_000, [0, 15_000], false);
		expect(FakeAudio.created).toHaveLength(1);
		expect(engine.getSnapshot()).toMatchObject({
			positionMs: 8_000,
			playing: true,
		});
	});

	test("a finalized fragment continues into the next one", () => {
		fakeBrowser();
		const { engine } = segmented([liveFragment(5_000)], 5_000);
		engine.startAt(0, true);
		FakeAudio.created[0]?.dispatchEvent(new Event("loadedmetadata"));
		const next: PlaybackSegment = {
			kind: "active_hls",
			start_ms: 7_000,
			end_ms: 9_000,
			audio_file_id: "fragment-2",
			media_url: "/api/audio/sessions/1/segments/fragment-2",
			hls_playlist_url: "/api/audio/sessions/1/live/fragment-2/playlist.m3u8",
		};
		const finalized: PlaybackSegment = {
			...liveFragment(7_000),
			kind: "audio",
			hls_playlist_url: null,
		};

		// The playlist ended before the manifest noticed: wait for it.
		FakeAudio.created[0]?.dispatchEvent(new Event("ended"));
		expect(FakeAudio.created).toHaveLength(1);
		engine.setTimeline([finalized, next], 9_000);
		expect(FakeAudio.created).toHaveLength(2);
		expect(engine.getSnapshot()).toMatchObject({
			positionMs: 7_000,
			playing: true,
		});
		expect(FakeAudio.created[1]?.src).toContain("fragment-2");
	});

	test("trailing silence plays to where the latest manifest ends it", () => {
		fakeBrowser();
		const silence = (end_ms: number): PlaybackSegment => ({
			kind: "silence",
			start_ms: 0,
			end_ms,
			reason: "active_silence",
		});
		const { engine } = segmented([silence(5_000)], 5_000);
		engine.startAt(4_000, true);
		frame(0);
		engine.setTimeline([silence(9_000)], 9_000);
		frame(2_000);
		expect(engine.getSnapshot()).toMatchObject({
			positionMs: 6_000,
			playing: true,
		});
		frame(5_000);
		expect(engine.getSnapshot()).toMatchObject({
			positionMs: 9_000,
			playing: false,
		});
	});
});

describe("SilenceFreeEngine", () => {
	test("starts from the duration it was given, before any metadata", () => {
		expect(silenceFree().getSnapshot()).toMatchObject({
			durationMs: 8_000,
			positionMs: 0,
			playing: false,
			retryKey: 0,
		});
	});

	test("the element's metadata replaces the duration and gets the settings", () => {
		const engine = silenceFree();
		engine.setVolume(0.5);
		engine.setPlaybackRate(1.25);
		const audio = { duration: 12, volume: 1, playbackRate: 1 };
		engine.mediaHandlers.onDurationChange(audio as HTMLAudioElement);
		expect(engine.getSnapshot().durationMs).toBe(12_000);
		expect(audio).toMatchObject({ volume: 0.5, playbackRate: 1.25 });
	});

	test("new media resets position, duration and errors; the same media does not", () => {
		const engine = silenceFree();
		engine.startAt(5_000, false);
		engine.setMedia("https://sakiot.test/mix.ogg", 8_000);
		expect(engine.getSnapshot().positionMs).toBe(5_000);
		engine.setMedia("https://sakiot.test/other.ogg", 3_000);
		expect(engine.getSnapshot()).toMatchObject({
			positionMs: 0,
			durationMs: 3_000,
			playbackError: null,
		});
	});

	test("a pause from the element stops playback", () => {
		const engine = silenceFree();
		engine.mediaHandlers.onPause();
		expect(engine.getSnapshot().playing).toBe(false);
	});
});
