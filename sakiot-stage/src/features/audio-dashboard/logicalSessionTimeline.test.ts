import { describe, expect, it } from "bun:test";
import type { SessionManifest } from "../../app/apiSlice";
import { normalizeSessionSegments } from "./logicalSessionTimeline";

function manifest(
	segments: SessionManifest["segments"],
	duration_ms = 8_000,
): SessionManifest {
	return {
		recording_session_id: "1",
		guild_id: "10",
		user_id: "20",
		starting_channel_id: "30",
		state: "finalized",
		started_at_ms: 1_000,
		ended_at_ms: 9_000,
		duration_ms,
		channel_journey: ["30", "31"],
		segments,
		events: [],
	};
}

describe("normalizeSessionSegments", () => {
	it("orders fragments and fills uncovered regions with synthetic silence", () => {
		const result = normalizeSessionSegments(
			manifest([
				{
					kind: "audio",
					start_ms: 5_000,
					end_ms: 7_000,
					channel_id: "31",
					media_url: "/second",
				},
				{
					kind: "audio",
					start_ms: 1_000,
					end_ms: 3_000,
					channel_id: "30",
					media_url: "/first",
				},
			]),
		);

		expect(
			result.map(({ kind, start_ms, end_ms }) => [kind, start_ms, end_ms]),
		).toEqual([
			["silence", 0, 1_000],
			["audio", 1_000, 3_000],
			["silence", 3_000, 5_000],
			["audio", 5_000, 7_000],
			["silence", 7_000, 8_000],
		]);
	});

	it("clips overlapping segments so every logical millisecond has one source", () => {
		const result = normalizeSessionSegments(
			manifest(
				[
					{ kind: "audio", start_ms: 0, end_ms: 4_000 },
					{ kind: "silence", start_ms: 3_000, end_ms: 5_000 },
					{
						kind: "active_hls",
						start_ms: 5_000,
						end_ms: 6_000,
						hls_playlist_url: "/live",
					},
				],
				6_000,
			),
		);

		expect(
			result.map(({ kind, start_ms, end_ms }) => [kind, start_ms, end_ms]),
		).toEqual([
			["audio", 0, 4_000],
			["silence", 4_000, 5_000],
			["active_hls", 5_000, 6_000],
		]);
	});
});
