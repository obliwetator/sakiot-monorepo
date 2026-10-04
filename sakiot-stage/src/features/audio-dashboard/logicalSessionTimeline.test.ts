import { describe, expect, it } from "bun:test";
import type { SessionManifest } from "../../app/apiSlice";
import {
	extendActiveManifest,
	isolateSessionChannel,
	normalizeSessionSegments,
} from "./logicalSessionTimeline";

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

	it("mutes other channels without changing logical positions", () => {
		const normalized = normalizeSessionSegments(
			manifest([
				{
					kind: "audio",
					start_ms: 0,
					end_ms: 3_000,
					channel_id: "30",
					media_url: "/first",
				},
				{
					kind: "audio",
					start_ms: 3_000,
					end_ms: 8_000,
					channel_id: "31",
					media_url: "/second",
				},
			]),
		);

		const result = isolateSessionChannel(normalized, "31");

		expect(
			result.map(({ kind, start_ms, end_ms, reason, media_url }) => ({
				kind,
				start_ms,
				end_ms,
				reason,
				media_url,
			})),
		).toEqual([
			{
				kind: "silence",
				start_ms: 0,
				end_ms: 3_000,
				reason: "channel_filtered",
				media_url: null,
			},
			{
				kind: "audio",
				start_ms: 3_000,
				end_ms: 8_000,
				reason: undefined,
				media_url: "/second",
			},
		]);
	});
});

describe("extendActiveManifest", () => {
	const recording = (): SessionManifest => ({
		...manifest(
			[
				{ kind: "audio", start_ms: 0, end_ms: 3_000, media_url: "/done" },
				{
					kind: "active_hls",
					start_ms: 4_000,
					end_ms: 8_000,
					media_url: "/live",
				},
			],
			8_000,
		),
		state: "active",
		ended_at_ms: null,
	});

	it("moves an active session's end and its recording fragment with the clock", () => {
		const extended = extendActiveManifest(recording(), 1_500);
		expect(extended.duration_ms).toBe(9_500);
		expect(extended.segments.map(({ kind, end_ms }) => [kind, end_ms])).toEqual(
			[
				["audio", 3_000],
				["active_hls", 9_500],
			],
		);
	});

	it("leaves finished, paused and freshly fetched sessions alone", () => {
		const active = recording();
		expect(extendActiveManifest(active, 0)).toBe(active);
		expect(extendActiveManifest(active, -200)).toBe(active);
		const paused = { ...active, state: "pending" };
		expect(extendActiveManifest(paused, 1_500)).toBe(paused);
		const finalized = manifest([]);
		expect(extendActiveManifest(finalized, 1_500)).toBe(finalized);
	});
});
