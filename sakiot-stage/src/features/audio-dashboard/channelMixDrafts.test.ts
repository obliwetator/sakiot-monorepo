import { beforeEach, describe, expect, test } from "bun:test";
import type {
	ChannelMixParticipantSettings,
	ChannelMixTrack,
} from "../../app/apiSlice";
import {
	channelMixRenderSettingsEqual,
	mergeChannelMixDraft,
	readChannelMixDraft,
	writeChannelMixDraft,
} from "./channelMixDrafts";

const storage = new Map<string, string>();
Object.defineProperty(globalThis, "localStorage", {
	configurable: true,
	value: {
		getItem: (key: string) => storage.get(key) ?? null,
		setItem: (key: string, value: string) => storage.set(key, value),
		removeItem: (key: string) => storage.delete(key),
	},
});

const tracks: ChannelMixTrack[] = [
	{ user_id: "2", display_name: "Two", is_anchor: true, segments: [] },
	{ user_id: "10", display_name: "Ten", is_anchor: false, segments: [] },
];

describe("channel mix drafts", () => {
	beforeEach(() => storage.clear());

	test("uses local settings first, then in-memory/server settings, then defaults", () => {
		const local: ChannelMixParticipantSettings = {
			user_id: "2",
			gain_db: 4,
			muted: true,
		};
		writeChannelMixDraft("session", [local]);

		const merged = mergeChannelMixDraft(
			"session",
			tracks,
			[
				{ user_id: "2", gain_db: -3, muted: false },
				{ user_id: "10", gain_db: 2, muted: true },
			],
			[{ user_id: "10", gain_db: 8, muted: false }],
		);

		expect(merged).toEqual([
			{ user_id: "10", gain_db: 8, muted: false },
			{ user_id: "2", gain_db: 4, muted: true },
		]);
	});

	test("defaults a newly discovered participant and clamps stored gains", () => {
		writeChannelMixDraft("session", [
			{ user_id: "2", gain_db: 99, muted: false },
		]);
		const merged = mergeChannelMixDraft("session", tracks, undefined);

		expect(merged.find((item) => item.user_id === "2")?.gain_db).toBe(12);
		expect(merged.find((item) => item.user_id === "10")).toEqual({
			user_id: "10",
			gain_db: 0,
			muted: false,
		});
	});

	test("compares settings independent of participant order", () => {
		expect(
			channelMixRenderSettingsEqual(
				[
					{ user_id: "10", gain_db: 1, muted: false },
					{ user_id: "2", gain_db: 0, muted: true },
				],
				[
					{ user_id: "2", gain_db: 0, muted: true },
					{ user_id: "10", gain_db: 1.00001, muted: false },
				],
			),
		).toBe(true);
		expect(readChannelMixDraft("missing")).toEqual([]);
	});
});
