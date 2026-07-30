import { describe, expect, it } from "bun:test";
import { buildStampPlaybackTarget } from "./stampNavigation";

describe("buildStampPlaybackTarget", () => {
	it("opens logical session using session-relative time", () => {
		expect(
			buildStampPlaybackTarget(
				{
					stamp_ts: 12_000,
					offset_ms: 500,
					recording_session_id: "373",
					session_started_at_ms: 1_000,
					channel_id: "20",
					year: 2026,
					month: 7,
					file_name: "10-user",
					start_ts: 10_000,
				},
				"10",
			),
		).toEqual({
			path: "/dashboard/10/audio/session/373?t=11.5",
			relativeSeconds: 11.5,
			scope: "session",
		});
	});

	it("falls back to fragment route for legacy files", () => {
		expect(
			buildStampPlaybackTarget(
				{
					stamp_ts: 12_000,
					offset_ms: 500,
					channel_id: "20",
					year: 2026,
					month: 7,
					file_name: "10-user name",
					start_ts: 10_000,
				},
				"10",
			),
		).toEqual({
			path: "/dashboard/10/audio/20/2026/7/10-user%20name?t=2.5",
			relativeSeconds: 2.5,
			scope: "fragment",
		});
	});

	it("returns null when neither session nor fragment location is complete", () => {
		expect(
			buildStampPlaybackTarget(
				{
					stamp_ts: 12_000,
					offset_ms: 0,
					channel_id: "20",
				},
				"10",
			),
		).toBeNull();
	});
});
