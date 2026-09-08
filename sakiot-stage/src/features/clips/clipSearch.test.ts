import { describe, expect, it } from "bun:test";
import type { ClipData } from "../../app/apiSlice";
import { filterClips } from "./clipSearch";

function clip(overrides: Partial<ClipData> & { clip_id: string }): ClipData {
	return {
		channel_id: "channel-1",
		guild_id: "guild-1",
		silence_free: false,
		start_time: 0,
		user_id: "user-1",
		...overrides,
	};
}

const clips: ClipData[] = [
	clip({ clip_id: "clip-1", name: "PAIKSTE TO GAME", user_id: "111" }),
	clip({
		clip_id: "clip-2",
		name: "tana",
		user_id: "222",
		original_file_name: "2026-08-08-222-tana.ogg",
	}),
	clip({ clip_id: "clip-3", name: null, user_id: "333" }),
];

describe("filterClips", () => {
	it("returns the list unchanged for an empty or blank query", () => {
		expect(filterClips(clips, "")).toBe(clips);
		expect(filterClips(clips, "   ")).toBe(clips);
	});

	it("matches the name case-insensitively", () => {
		expect(filterClips(clips, "paikste").map((c) => c.clip_id)).toEqual([
			"clip-1",
		]);
		expect(filterClips(clips, "TANA").map((c) => c.clip_id)).toEqual([
			"clip-2",
		]);
	});

	it("matches a partial name", () => {
		expect(filterClips(clips, "gam").map((c) => c.clip_id)).toEqual(["clip-1"]);
	});

	it("matches the user id", () => {
		expect(filterClips(clips, "222").map((c) => c.clip_id)).toEqual(["clip-2"]);
	});

	it("matches the clip id", () => {
		expect(filterClips(clips, "clip-3").map((c) => c.clip_id)).toEqual([
			"clip-3",
		]);
	});

	it("matches the original file name", () => {
		expect(filterClips(clips, "2026-08-08").map((c) => c.clip_id)).toEqual([
			"clip-2",
		]);
	});

	it("returns nothing when no clip matches", () => {
		expect(filterClips(clips, "no-such-clip")).toEqual([]);
	});
});
