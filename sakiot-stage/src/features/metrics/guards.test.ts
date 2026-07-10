import { describe, expect, it } from "bun:test";
import { parseGuildVoicePayload, parseMetricsPayload } from "./guards";
import type { GuildVoicePayload, Metrics, VoiceState } from "./types";

function metrics(overrides: Partial<Metrics> = {}): Metrics {
	return {
		total_guilds: 2,
		active_voice_connections: 1,
		uptime_seconds: 3600,
		commands_executed: 10,
		messages_received: 100,
		guilds: [{ id: "1", name: "Guild" }],
		active_recordings: 1,
		ffmpeg_spawn_failures: 0,
		ffmpeg_process_crashes: 0,
		audio_packets_received: 5000,
		audio_packets_dropped: 2,
		gateway_reconnects: 0,
		driver_reconnects: 0,
		voice_state_updates_received: 40,
		db_query_errors: 0,
		db_insert_failures: 0,
		grpc_active_streams: 1,
		process_rss_bytes: 1024,
		process_open_fds: 32,
		tokio_active_tasks: 12,
		...overrides,
	};
}

function voiceState(overrides: Partial<VoiceState> = {}): VoiceState {
	return {
		user_id: "user-1",
		channel_id: "channel-1",
		mute: false,
		deaf: false,
		self_mute: false,
		self_deaf: false,
		self_stream: false,
		self_video: false,
		suppress: false,
		...overrides,
	};
}

function guildVoicePayload(
	overrides: Partial<GuildVoicePayload> = {},
): GuildVoicePayload {
	return {
		voice_states: [voiceState()],
		user_start_times: { "user-1": 1700000000 },
		...overrides,
	};
}

describe("parseMetricsPayload", () => {
	it("accepts a well-formed payload", () => {
		const parsed = parseMetricsPayload(JSON.stringify(metrics()));
		expect(parsed?.uptime_seconds).toBe(3600);
		expect(parsed?.guilds).toEqual([{ id: "1", name: "Guild" }]);
	});

	it("accepts an optional last_voice_packet_time number", () => {
		const payload = JSON.stringify(metrics({ last_voice_packet_time: 42 }));
		expect(parseMetricsPayload(payload)?.last_voice_packet_time).toBe(42);
	});

	it("rejects invalid JSON and non-objects", () => {
		expect(parseMetricsPayload("not json")).toBeNull();
		expect(parseMetricsPayload("42")).toBeNull();
		expect(parseMetricsPayload("null")).toBeNull();
	});

	it("rejects wrongly typed fields", () => {
		const bad = { ...metrics(), uptime_seconds: "3600" };
		expect(parseMetricsPayload(JSON.stringify(bad))).toBeNull();
	});

	it("rejects malformed guild entries", () => {
		const bad = { ...metrics(), guilds: [{ id: 1, name: "Guild" }] };
		expect(parseMetricsPayload(JSON.stringify(bad))).toBeNull();
	});

	it("rejects missing fields", () => {
		const { total_guilds: _dropped, ...bad } = metrics();
		expect(parseMetricsPayload(JSON.stringify(bad))).toBeNull();
	});
});

describe("parseGuildVoicePayload", () => {
	it("accepts a payload without recording metrics", () => {
		const parsed = parseGuildVoicePayload(JSON.stringify(guildVoicePayload()));
		expect(parsed?.voice_states).toHaveLength(1);
		expect(parsed?.user_start_times["user-1"]).toBe(1700000000);
	});

	it("accepts a null or well-formed recording_metrics", () => {
		const withNull = guildVoicePayload({ recording_metrics: null });
		expect(parseGuildVoicePayload(JSON.stringify(withNull))).not.toBeNull();

		const withMetrics = guildVoicePayload({
			recording_metrics: {
				active_recordings: 1,
				ffmpeg_spawn_failures: 0,
				ffmpeg_process_crashes: 0,
				audio_packets_received: 100,
				audio_packets_dropped: 0,
			},
		});
		const parsed = parseGuildVoicePayload(JSON.stringify(withMetrics));
		expect(parsed?.recording_metrics?.active_recordings).toBe(1);
	});

	it("rejects malformed voice states", () => {
		const bad = guildVoicePayload({
			voice_states: [{ ...voiceState(), mute: "yes" } as unknown as VoiceState],
		});
		expect(parseGuildVoicePayload(JSON.stringify(bad))).toBeNull();
	});

	it("rejects non-numeric start times", () => {
		const bad = guildVoicePayload({
			user_start_times: { "user-1": "soon" } as unknown as Record<
				string,
				number
			>,
		});
		expect(parseGuildVoicePayload(JSON.stringify(bad))).toBeNull();
	});

	it("rejects invalid JSON", () => {
		expect(parseGuildVoicePayload("{oops")).toBeNull();
	});
});
