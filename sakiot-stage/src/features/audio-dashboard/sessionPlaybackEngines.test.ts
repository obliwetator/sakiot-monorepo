import { describe, expect, test } from "bun:test";
import type { PlaybackSegment } from "./logicalSessionTimeline";
import { SegmentedSessionEngine } from "./segmentedSessionEngine";
import { SilenceFreeEngine } from "./silenceFreeEngine";

// Only the paths that never start media are covered here: playback itself
// needs a browser and is exercised by the Playwright suites.

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
		engine.setDurationMs(6_000);
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
