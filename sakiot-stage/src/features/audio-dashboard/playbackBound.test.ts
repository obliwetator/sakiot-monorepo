import { describe, expect, test } from "bun:test";
import { type BoundedPlayback, PlaybackBound } from "./playbackBound";

function harness(initial: { positionMs?: number; playing?: boolean } = {}) {
	const calls: string[] = [];
	const state = {
		positionMs: initial.positionMs ?? 0,
		playing: initial.playing ?? false,
		durationMs: 10_000,
		active: false,
	};
	const playback: BoundedPlayback = {
		position: () => state.positionMs,
		isPlaying: () => state.playing,
		duration: () => state.durationMs,
		startAt: (positionMs, autoplay) => {
			calls.push(`startAt ${positionMs} ${autoplay}`);
			state.positionMs = positionMs;
			state.playing = autoplay;
		},
		stop: () => {
			calls.push("stop");
			state.playing = false;
		},
		loopDisabled: () => calls.push("loopDisabled"),
		beforePlay: () => calls.push("beforePlay"),
		clearSeekPreview: () => calls.push("clearSeekPreview"),
		activeChanged: (active) => {
			state.active = active;
		},
	};
	return { bound: new PlaybackBound(playback), calls, state };
}

describe("PlaybackBound", () => {
	test("looped playback starts inside the selection and loops at its end", () => {
		const { bound, calls, state } = harness({ positionMs: 500 });
		bound.togglePlay([1_000, 2_000], true);
		expect(calls).toEqual(["beforePlay", "startAt 1000 true"]);
		expect(state.active).toBe(true);
		expect(bound.apply(1_999)).toBe(false);
		expect(bound.apply(2_000)).toBe(true);
		expect(calls.at(-1)).toBe("startAt 1000 true");
	});

	test("a preview stops at the selection end and disarms", () => {
		const { bound, calls, state } = harness();
		bound.togglePreview([1_000, 2_000], false);
		expect(calls).toEqual(["clearSeekPreview", "startAt 1000 true"]);
		expect(bound.apply(2_500)).toBe(true);
		expect(calls.at(-1)).toBe("startAt 2000 false");
		expect(state.active).toBe(false);
		expect(bound.apply(3_000)).toBe(false);
	});

	test("playing from the end restarts at zero without a bound", () => {
		const { bound, calls, state } = harness({ positionMs: 10_000 });
		bound.togglePlay([0, 0], false);
		expect(calls).toEqual(["beforePlay", "startAt 0 true"]);
		expect(state.active).toBe(false);
	});

	test("toggling while playing only stops", () => {
		const { bound, calls } = harness({ playing: true });
		bound.togglePlay([0, 1_000], true);
		expect(calls).toEqual(["stop"]);
	});

	test("seeking outside a looped selection disables the loop", () => {
		const { bound, calls, state } = harness({ positionMs: 1_500 });
		bound.togglePlay([1_000, 2_000], true);
		bound.prepareSeek([1_000, 2_000], true, 5_000);
		expect(state.active).toBe(false);
		expect(calls.at(-1)).toBe("loopDisabled");
	});

	test("a selection that no longer holds the playhead drops the bound", () => {
		const { bound, calls, state } = harness();
		bound.togglePreview([1_000, 2_000], true);
		state.positionMs = 1_500;
		bound.sync([3_000, 4_000], true);
		expect(state.active).toBe(false);
		expect(calls.at(-1)).toBe("loopDisabled");
	});

	test("turning the loop off makes the bound stop instead of looping", () => {
		const { bound, calls } = harness();
		bound.togglePreview([1_000, 2_000], true);
		bound.updateLoop(false, [1_000, 2_000]);
		bound.apply(2_000);
		expect(calls.at(-1)).toBe("startAt 2000 false");
	});
});
