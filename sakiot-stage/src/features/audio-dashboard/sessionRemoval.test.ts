import { describe, expect, it } from "bun:test";
import type { Channels } from "../../Constants";
import { sessionRemoval } from "./sessionRemoval";

const erasing = {
	erasure: { id: "erasure-1", state: "grace", purge_at_ms: 1 },
	playable: true,
};

const tree: Channels[] = [
	{
		channel_id: "channel-a",
		dirs: [
			{
				year: 2026,
				months: {
					9: [
						{ file: "visible.ogg", recording_session_id: "1" },
						{
							file: "removed.ogg",
							recording_session_id: "2",
							removal: erasing,
						},
					],
				},
			},
		],
	},
	{
		channel_id: "channel-b",
		dirs: [
			{
				year: 2026,
				months: {
					10: [
						{
							file: "server.ogg",
							recording_session_id: "3",
							removal: { playable: false },
						},
					],
				},
			},
		],
	},
];

describe("sessionRemoval", () => {
	it("finds a session's removal in any channel and month", () => {
		expect(sessionRemoval(tree, "2")).toEqual(erasing);
		expect(sessionRemoval(tree, "3")).toEqual({ playable: false });
	});

	it("is null for visible, unlisted and not yet loaded sessions", () => {
		expect(sessionRemoval(tree, "1")).toBeNull();
		expect(sessionRemoval(tree, "4")).toBeNull();
		expect(sessionRemoval(undefined, "2")).toBeNull();
	});
});
