import { describe, expect, it } from "bun:test";
import {
	type AudioTimelineEvent,
	buildEventTimelineModel,
	clusterTimelinePoints,
	formatTimelineOffset,
} from "./eventTimelineModel";

function event(
	event_type: string,
	offset_ms: number,
	extra: Partial<AudioTimelineEvent> = {},
): AudioTimelineEvent {
	return { event_type, offset_ms, ...extra };
}

describe("buildEventTimelineModel", () => {
	it("pairs toggle events into state intervals regardless of event casing", () => {
		const model = buildEventTimelineModel(
			[event("SELF_MUTE", 60_000), event("self_unmute", 180_000)],
			240_000,
		);

		expect(model.totalEvents).toBe(2);
		expect(model.lanes).toHaveLength(1);
		expect(model.lanes[0].id).toBe("mute");
		expect(model.lanes[0].points).toEqual([]);
		expect(model.lanes[0].intervals).toMatchObject([
			{
				startMs: 60_000,
				endMs: 180_000,
				label: "Muted",
				track: 1,
				startsAtBoundary: false,
				endsAtBoundary: false,
			},
		]);
	});

	it("extends state to timeline boundary when matching transition is outside recording", () => {
		const model = buildEventTimelineModel(
			[event("server_unmute", 5_000), event("self_mute", 8_000)],
			10_000,
		);
		const muteLane = model.lanes.find((lane) => lane.id === "mute");

		expect(muteLane?.intervals).toMatchObject([
			{
				startMs: 0,
				endMs: 5_000,
				label: "Server muted",
				startsAtBoundary: true,
				endsAtBoundary: false,
			},
			{
				startMs: 8_000,
				endMs: 10_000,
				label: "Muted",
				startsAtBoundary: false,
				endsAtBoundary: true,
			},
		]);
	});

	it("uses fixed semantic lanes for session, channel, connection, and unknown events", () => {
		const model = buildEventTimelineModel(
			[
				event("network_pause", 1_000, { source: "recording" }),
				event("channel_switch", 1_500, { source: "voice_state" }),
				event("handoff:failed", 1_600, { source: "voice_connection" }),
				event("mystery_event", 1_700),
				event("resume", 2_000, { source: "recording" }),
			],
			3_000,
		);

		expect(model.lanes.map((lane) => lane.id)).toEqual([
			"recording",
			"channel",
			"connection",
			"other",
		]);
		expect(model.lanes[0].intervals).toMatchObject([
			{ startMs: 1_000, endMs: 2_000, label: "Network pause" },
		]);
		expect(model.lanes[2].points[0].color).toBe("#ef4444");
	});

	it("drops events outside playable timeline", () => {
		const model = buildEventTimelineModel(
			[
				event("channel_join", -1),
				event("channel_switch", 500),
				event("channel_leave", 1_001),
			],
			1_000,
		);

		expect(model.totalEvents).toBe(1);
		expect(model.lanes[0].points).toHaveLength(1);
	});
});

describe("clusterTimelinePoints", () => {
	it("clusters only milestones that collide in screen space", () => {
		const model = buildEventTimelineModel(
			[
				event("channel_join", 1_000),
				event("channel_switch", 1_050),
				event("channel_leave", 3_000),
			],
			10_000,
		);
		const points = model.lanes.flatMap((lane) => lane.points);

		const clusters = clusterTimelinePoints(points, 10_000, 1_000, 14);

		expect(clusters.map((cluster) => cluster.points.length)).toEqual([2, 1]);
		expect(clusters[0]).toMatchObject({
			offsetMs: 1_025,
			startMs: 1_000,
			endMs: 1_050,
		});
	});
});

describe("formatTimelineOffset", () => {
	it("does not wrap recordings longer than one day", () => {
		expect(formatTimelineOffset(27 * 3_600_000 + 2 * 60_000 + 3_000)).toBe(
			"27:02:03",
		);
	});
});
