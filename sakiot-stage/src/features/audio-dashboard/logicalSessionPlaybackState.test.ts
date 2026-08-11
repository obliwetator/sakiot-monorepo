import { describe, expect, test } from "bun:test";
import {
	clampPlaybackPosition,
	isSameMediaSegment,
	parseSessionDeepLink,
	segmentAtPosition,
	selectionForTab,
	shouldRetryMediaLoad,
} from "./logicalSessionPlaybackState";
import type { PlaybackSegment } from "./logicalSessionTimeline";
import { parseSilenceRemovalStatus } from "./silenceRemovalState";

const audio = (
	id: number,
	start_ms: number,
	end_ms: number,
): PlaybackSegment => ({
	kind: "audio",
	start_ms,
	end_ms,
	media_url: `/audio/${id}`,
	audio_file_id: id,
});

describe("session deep links", () => {
	test("parses stamp and silence-free timeline flags", () => {
		expect(
			parseSessionDeepLink("?t=12.5&clip=stamp&timeline=silence-free"),
		).toEqual({ positionMs: 12_500, fromStamp: true, silenceFree: true });
	});

	test("rejects missing, negative, and nonnumeric positions", () => {
		expect(parseSessionDeepLink("?clip=stamp")).toBeNull();
		expect(parseSessionDeepLink("?t=-1")).toBeNull();
		expect(parseSessionDeepLink("?t=nope")).toBeNull();
	});
});

describe("playback state helpers", () => {
	test("selects segments with end-exclusive bounds", () => {
		const segments = [audio(1, 0, 1_000), audio(2, 1_000, 2_000)];
		expect(segmentAtPosition(segments, 999)?.audio_file_id).toBe(1);
		expect(segmentAtPosition(segments, 1_000)?.audio_file_id).toBe(2);
		expect(segmentAtPosition(segments, 2_000)).toBeUndefined();
	});

	test("clamps seeks to session bounds", () => {
		expect(clampPlaybackPosition(-5, 100)).toBe(0);
		expect(clampPlaybackPosition(50, 100)).toBe(50);
		expect(clampPlaybackPosition(500, 100)).toBe(100);
	});

	test("recognizes same physical media across state refresh", () => {
		expect(isSameMediaSegment(audio(1, 0, 1_000), audio(1, 0, 2_000))).toBe(
			true,
		);
		expect(isSameMediaSegment(audio(1, 0, 1_000), audio(2, 0, 1_000))).toBe(
			false,
		);
	});

	test("allows exactly one auth retry until successful load resets it", () => {
		expect(shouldRetryMediaLoad(false, true)).toBe(true);
		expect(shouldRetryMediaLoad(true, true)).toBe(false);
		expect(shouldRetryMediaLoad(false, false)).toBe(false);
	});

	test("reconciles tab selection without sharing mutable tuples", () => {
		const normal: [number, number] = [1_000, 2_000];
		expect(selectionForTab("normal", normal, null, 5_000)).toBe(normal);
		expect(selectionForTab("silence", normal, null, 5_000)).toEqual([0, 5_000]);
		const silence: [number, number] = [500, 1_500];
		expect(selectionForTab("silence", normal, silence, 5_000)).toBe(silence);
	});
});

describe("silence-removal status parsing", () => {
	test("clamps finite progress and preserves known states", () => {
		expect(
			parseSilenceRemovalStatus({ status: "processing", progress: 120.2 }),
		).toEqual({ status: "processing", progress: 100 });
		expect(
			parseSilenceRemovalStatus({ status: "ready", progress: 99.6 }),
		).toEqual({ status: "ready", progress: 100 });
	});

	test("normalizes malformed server values", () => {
		expect(parseSilenceRemovalStatus(null)).toEqual({
			status: "failed",
			progress: 0,
		});
		expect(
			parseSilenceRemovalStatus({ status: "unknown", progress: Number.NaN }),
		).toEqual({ status: "idle", progress: 0 });
	});
});
