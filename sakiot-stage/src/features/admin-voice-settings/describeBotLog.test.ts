import { describe, expect, test } from "bun:test";
import type { BotLogEntry } from "../../app/apiSlice";
import { culprit, happened } from "./describeBotLog";

function entry(overrides: Partial<BotLogEntry> = {}): BotLogEntry {
	return {
		id: 1,
		kind: "deafened",
		occurred_at_ms: 0,
		channel_name: "General",
		to_channel_name: null,
		actor_id: "600",
		actor_name: "kavliara",
		attribution: "exact",
		...overrides,
	};
}

describe("happened", () => {
	test("says what was done and where", () => {
		expect(happened(entry())).toBe("Deafened in General");
		expect(happened(entry({ kind: "undeafened" }))).toBe(
			"Undeafened in General",
		);
		expect(happened(entry({ kind: "moved", to_channel_name: "Music" }))).toBe(
			"Moved from General to Music",
		);
		expect(happened(entry({ kind: "disconnected" }))).toBe(
			"Disconnected from General",
		);
	});

	test("says what to check when the cleanup fails", () => {
		expect(happened(entry({ kind: "deafen_clear_failed" }))).toContain(
			"Deafen Members",
		);
	});

	test("copes with a channel that is gone", () => {
		expect(happened(entry({ channel_name: null }))).toBe(
			"Deafened in a deleted channel",
		);
	});
});

describe("culprit", () => {
	test("is sure only when Discord is", () => {
		expect(culprit(entry())).toBe("By kavliara");
		expect(culprit(entry({ attribution: "likely" }))).toBe(
			"Probably by kavliara",
		);
	});

	test("falls back to the id of a user never seen", () => {
		expect(culprit(entry({ actor_name: null }))).toBe("By User 600");
	});

	test("says why nobody is named", () => {
		const nobody = { actor_id: null, actor_name: null };
		expect(culprit(entry({ ...nobody, attribution: "unknown" }))).toBe(
			"Discord's audit log doesn't say who",
		);
		expect(culprit(entry({ ...nobody, attribution: "no_access" }))).toContain(
			"View Audit Log",
		);
	});

	test("names nobody for a failed cleanup", () => {
		expect(culprit(entry({ kind: "deafen_clear_failed" }))).toBeNull();
	});
});
