import { describe, expect, test } from "bun:test";
import { ChannelMixEngine } from "./channelMixEngine";

function mixEngine(durationMs = 10_000) {
	return new ChannelMixEngine({
		tracks: [],
		durationMs,
		settings: [],
		volume: 1,
		playbackRate: 1,
	});
}

describe("ChannelMixEngine", () => {
	test("seek clamps to the mix and stops following live", () => {
		const mix = mixEngine();
		mix.seek(-5);
		expect(mix.getSnapshot().positionMs).toBe(0);
		mix.seek(25_000);
		expect(mix.getSnapshot()).toMatchObject({
			positionMs: 10_000,
			followingLive: false,
			playing: false,
		});
	});

	test("a shorter mix pulls the playhead back inside it", () => {
		const mix = mixEngine();
		mix.seek(8_000);
		mix.setDurationMs(5_000);
		expect(mix.getSnapshot().positionMs).toBe(5_000);
	});

	test("the snapshot keeps its identity until a value changes", () => {
		const mix = mixEngine();
		const before = mix.getSnapshot();
		mix.seek(0);
		mix.setVolume(0.5);
		mix.setPlaybackRate(1);
		expect(mix.getSnapshot()).toBe(before);
		mix.seek(1_000);
		expect(mix.getSnapshot()).not.toBe(before);
	});

	test("listeners hear each change until they unsubscribe", () => {
		const mix = mixEngine();
		let calls = 0;
		const unsubscribe = mix.subscribe(() => {
			calls += 1;
		});
		mix.seek(1_000);
		mix.seek(1_000);
		unsubscribe();
		mix.seek(2_000);
		expect(calls).toBe(1);
	});

	test("dispose leaves the engine usable, as Strict Mode remounts need", () => {
		const mix = mixEngine();
		mix.dispose();
		mix.seek(2_000);
		expect(mix.getSnapshot()).toMatchObject({
			positionMs: 2_000,
			playing: false,
		});
	});
});
