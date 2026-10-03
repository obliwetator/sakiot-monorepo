import { describe, expect, test } from "bun:test";
import type { components } from "../api/openapi";
import { patchSession } from "./patchTree";

type Channels = components["schemas"]["Channels"];

const file = (session: string, state = "finalized") => ({
	file: `${session}.ogg`,
	recording_session_id: session,
	state,
});

const entry = (
	channel: string,
	year: number,
	month: string,
	session: string,
	state = "finalized",
): Channels => ({
	channel_id: channel,
	dirs: [{ year, months: { [month]: [file(session, state)] } }],
});

function tree(): Channels[] {
	return [
		{
			channel_id: "9007199254740993",
			dirs: [
				{ year: 2026, months: { "10": [file("3"), file("2"), file("1")] } },
				{ year: 2025, months: { "12": [file("0")] } },
			],
		},
	];
}

describe("patchSession", () => {
	test("an updated session keeps its place", () => {
		const t = tree();
		patchSession(t, "2", entry("9007199254740993", 2026, "10", "2", "active"));
		expect(t[0]?.dirs[0]?.months?.["10"]).toEqual([
			file("3"),
			file("2", "active"),
			file("1"),
		]);
	});

	test("a new session goes first in its month, creating groups as needed", () => {
		const t = tree();
		patchSession(t, "4", entry("9007199254740993", 2026, "10", "4"));
		expect(t[0]?.dirs[0]?.months?.["10"]?.[0]).toEqual(file("4"));

		// A new channel is placed in id order, compared as integers.
		patchSession(t, "5", entry("10", 2027, "1", "5"));
		patchSession(t, "6", entry("9007199254740995", 2026, "1", "6"));
		expect(t.map((c) => c.channel_id)).toEqual([
			"10",
			"9007199254740993",
			"9007199254740995",
		]);
		// A new year is placed newest first.
		patchSession(t, "7", entry("9007199254740993", 2027, "2", "7"));
		expect(t[1]?.dirs.map((d) => d.year)).toEqual([2027, 2026, 2025]);
	});

	test("a session that disappears is removed and empty groups pruned", () => {
		const t = tree();
		patchSession(t, "0", null);
		expect(t[0]?.dirs.map((d) => d.year)).toEqual([2026]);
		for (const id of ["1", "2", "3"]) patchSession(t, id, null);
		expect(t).toEqual([]);
	});

	test("an unknown session that is not visible changes nothing", () => {
		const t = tree();
		patchSession(t, "99", null);
		expect(t).toEqual(tree());
	});
});
