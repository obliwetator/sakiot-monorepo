export interface GuildInfo {
	id: string;
	name: string;
}

export interface Metrics {
	total_guilds: number;
	active_voice_connections: number;
	uptime_seconds: number;
	commands_executed: number;
	messages_received: number;
	guilds: GuildInfo[];
	active_recordings: number;
	ffmpeg_spawn_failures: number;
	ffmpeg_process_crashes: number;
	audio_packets_received: number;
	audio_packets_dropped: number;
	last_voice_packet_time?: number;
	gateway_reconnects: number;
	driver_reconnects: number;
	voice_state_updates_received: number;
	db_query_errors: number;
	db_insert_failures: number;
	grpc_active_streams: number;
	process_rss_bytes: number;
	process_open_fds: number;
	tokio_active_tasks: number;
}

export interface RecordingMetrics {
	active_recordings: number;
	ffmpeg_spawn_failures: number;
	ffmpeg_process_crashes: number;
	audio_packets_received: number;
	audio_packets_dropped: number;
	last_voice_packet_time?: number;
}

export interface VoiceState {
	user_id: string;
	channel_id: string;
	mute: boolean;
	deaf: boolean;
	self_mute: boolean;
	self_deaf: boolean;
	self_stream: boolean;
	self_video: boolean;
	suppress: boolean;
}
