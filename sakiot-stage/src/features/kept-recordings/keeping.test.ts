import { describe, expect, test } from "bun:test";
import {
	keepBlocked,
	keepers,
	recordingLabel,
	recordings,
	removal,
	serverUsage,
	viewerUsage,
} from "./keeping";

describe("usage", () => {
	test("says how much of the server's cap is kept", () => {
		expect(serverUsage({ cap: 0, kept: 0 })).toBe(
			"This server can't keep recordings yet: the bot's operators decide how many.",
		);
		expect(serverUsage({ cap: 1, kept: 0 })).toBe(
			"This server keeps 0 of the 1 recording it may keep.",
		);
		// Over a lowered cap, what is kept still shows.
		expect(serverUsage({ cap: 0, kept: 2 })).toBe(
			"This server keeps 2 of the 0 recordings it may keep.",
		);
	});

	test("says how much of their allowance the viewer keeps", () => {
		expect(viewerUsage({ allowance: null, used: 0 })).toBe(
			"You may keep recordings until the server's cap is reached.",
		);
		expect(viewerUsage({ allowance: null, used: 1 })).toBe(
			"You keep 1 recording, and may keep more until the server's cap is reached.",
		);
		expect(viewerUsage({ allowance: 0, used: 0 })).toBe(
			"You may not keep recordings here.",
		);
		expect(viewerUsage({ allowance: 3, used: 2 })).toBe(
			"You keep 2 of the 3 recordings you may keep.",
		);
	});
});

describe("keepBlocked", () => {
	test("names the first limit that stops another keep", () => {
		const room = { allowance: 2, used: 1, cap: 5, kept: 1 };
		expect(keepBlocked(room)).toBeNull();
		expect(keepBlocked({ ...room, allowance: null })).toBeNull();
		expect(keepBlocked({ ...room, used: 2 })).toBe(
			"You keep all the recordings you may keep. Stop keeping one to keep another.",
		);
		expect(keepBlocked({ ...room, allowance: 0, used: 0 })).toBe(
			"You may not keep recordings here.",
		);
		expect(keepBlocked({ ...room, kept: 5 })).toBe(
			"This server keeps all the recordings it may keep.",
		);
		expect(keepBlocked({ ...room, cap: 0, kept: 0 })).toBe(
			"This server can't keep recordings yet: the bot's operators decide how many.",
		);
	});
});

describe("labels", () => {
	test("name a recording by its speaker and channel, or what is left of them", () => {
		expect(
			recordingLabel({
				speaker_id: "7",
				speaker_name: "Ada",
				channel_id: "100",
				channel_name: "lounge",
			}),
		).toBe("Ada in lounge");
		expect(
			recordingLabel({
				speaker_id: "7",
				speaker_name: null,
				channel_id: "100",
				channel_name: null,
			}),
		).toBe("User 7 in channel 100");
	});

	test("say who keeps it and why it is gone", () => {
		expect(keepers({ mine: true, keepers: 1 })).toBe("Kept by you");
		expect(keepers({ mine: true, keepers: 3 })).toBe(
			"Kept by you and 2 others",
		);
		expect(keepers({ mine: false, keepers: 1 })).toBe("Kept by 1 member");
		expect(
			removal({ reason: "retention", removed_at_ms: 0 }).startsWith(
				"Removed by retention on ",
			),
		).toBe(true);
		expect(
			removal({ reason: "manager", removed_at_ms: 0 }).startsWith(
				"Removed by a manager on ",
			),
		).toBe(true);
		expect(recordings(1)).toBe("1 recording");
		expect(recordings(2)).toBe("2 recordings");
	});
});
