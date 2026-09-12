import { describe, expect, test } from "bun:test";
import type { ClipData } from "../../app/apiSlice";
import { clipAbsoluteStartMs, recordingStartFromStem } from "./clipStart";

function clip(overrides: Partial<ClipData> = {}): ClipData {
	return {
		channel_id: "channel-1",
		clip_id: "clip-1",
		guild_id: "guild-1",
		silence_free: false,
		start_time: 4,
		user_id: "user-1",
		...overrides,
	} as ClipData;
}

describe("recordingStartFromStem", () => {
	test("reads the epoch prefix of a recording stem", () => {
		expect(recordingStartFromStem("1700000000000-42")).toBe(1_700_000_000_000);
	});

	test("rejects stems without a numeric timestamp", () => {
		expect(recordingStartFromStem(null)).toBeNull();
		expect(recordingStartFromStem(undefined)).toBeNull();
		expect(recordingStartFromStem("")).toBeNull();
		expect(recordingStartFromStem("session:12")).toBeNull();
		expect(recordingStartFromStem("session-silence-free:12")).toBeNull();
		expect(recordingStartFromStem("-42")).toBeNull();
	});
});

describe("clipAbsoluteStartMs", () => {
	test("prefers the session start when the clip belongs to one", () => {
		// `session:12` has no timestamp of its own; parseInt used to read it as
		// NaN and hide the absolute time entirely.
		expect(
			clipAbsoluteStartMs(
				clip({ original_file_name: "session:12", start_time: 4 }),
				1_700_000_000_000,
			),
		).toBe(1_700_000_004_000);
	});

	test("falls back to the source stem for single-recording clips", () => {
		expect(
			clipAbsoluteStartMs(
				clip({ original_file_name: "1700000000000-42", start_time: 2.5 }),
				null,
			),
		).toBe(1_700_000_002_500);
	});

	test("returns null when no base time is known", () => {
		expect(clipAbsoluteStartMs(null, null)).toBeNull();
		expect(
			clipAbsoluteStartMs(clip({ original_file_name: "renamed clip" }), null),
		).toBeNull();
	});
});
