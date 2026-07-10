import type {
	GuildVoicePayload,
	Metrics,
	RecordingMetrics,
	VoiceState,
} from "./types";

// Websocket payloads are JSON from the server with no generated schema
// backing them (unlike the OpenAPI REST types), so validate the shape at
// runtime before trusting the annotation.

const METRICS_NUMBER_FIELDS = [
	"total_guilds",
	"active_voice_connections",
	"uptime_seconds",
	"commands_executed",
	"messages_received",
	"active_recordings",
	"ffmpeg_spawn_failures",
	"ffmpeg_process_crashes",
	"audio_packets_received",
	"audio_packets_dropped",
	"gateway_reconnects",
	"driver_reconnects",
	"voice_state_updates_received",
	"db_query_errors",
	"db_insert_failures",
	"grpc_active_streams",
	"process_rss_bytes",
	"process_open_fds",
	"tokio_active_tasks",
] as const satisfies readonly (keyof Metrics)[];

const RECORDING_METRICS_NUMBER_FIELDS = [
	"active_recordings",
	"ffmpeg_spawn_failures",
	"ffmpeg_process_crashes",
	"audio_packets_received",
	"audio_packets_dropped",
] as const satisfies readonly (keyof RecordingMetrics)[];

const VOICE_STATE_BOOLEAN_FIELDS = [
	"mute",
	"deaf",
	"self_mute",
	"self_deaf",
	"self_stream",
	"self_video",
	"suppress",
] as const satisfies readonly (keyof VoiceState)[];

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null;
}

function isOptionalNumber(value: unknown): boolean {
	return value === undefined || typeof value === "number";
}

function isGuildInfo(value: unknown): boolean {
	return (
		isRecord(value) &&
		typeof value.id === "string" &&
		typeof value.name === "string"
	);
}

export function isMetrics(value: unknown): value is Metrics {
	return (
		isRecord(value) &&
		Array.isArray(value.guilds) &&
		value.guilds.every(isGuildInfo) &&
		isOptionalNumber(value.last_voice_packet_time) &&
		METRICS_NUMBER_FIELDS.every((field) => typeof value[field] === "number")
	);
}

function isRecordingMetrics(value: unknown): value is RecordingMetrics {
	return (
		isRecord(value) &&
		isOptionalNumber(value.last_voice_packet_time) &&
		RECORDING_METRICS_NUMBER_FIELDS.every(
			(field) => typeof value[field] === "number",
		)
	);
}

function isVoiceState(value: unknown): value is VoiceState {
	return (
		isRecord(value) &&
		typeof value.user_id === "string" &&
		typeof value.channel_id === "string" &&
		VOICE_STATE_BOOLEAN_FIELDS.every(
			(field) => typeof value[field] === "boolean",
		)
	);
}

export function isGuildVoicePayload(
	value: unknown,
): value is GuildVoicePayload {
	if (!isRecord(value)) {
		return false;
	}
	const { voice_states, user_start_times, recording_metrics } = value;
	return (
		Array.isArray(voice_states) &&
		voice_states.every(isVoiceState) &&
		isRecord(user_start_times) &&
		Object.values(user_start_times).every((n) => typeof n === "number") &&
		(recording_metrics === undefined ||
			recording_metrics === null ||
			isRecordingMetrics(recording_metrics))
	);
}

function parsePayload<T>(
	payload: string,
	guard: (value: unknown) => value is T,
): T | null {
	try {
		const data: unknown = JSON.parse(payload);
		return guard(data) ? data : null;
	} catch {
		return null;
	}
}

export function parseMetricsPayload(payload: string): Metrics | null {
	return parsePayload(payload, isMetrics);
}

export function parseGuildVoicePayload(
	payload: string,
): GuildVoicePayload | null {
	return parsePayload(payload, isGuildVoicePayload);
}
