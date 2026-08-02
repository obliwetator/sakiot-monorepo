export interface StampPlaybackData {
	stamp_ts: number;
	offset_ms: number;
	recording_session_id?: string | null;
	session_started_at_ms?: number | null;
	channel_id: string;
	year?: number | null;
	month?: number | null;
	file_name?: string | null;
	start_ts?: number | null;
}

export interface StampPlaybackTarget {
	path: string;
	relativeSeconds: number;
	scope: "session" | "fragment";
}

export function buildStampPlaybackTarget(
	stamp: StampPlaybackData,
	guildId: string,
): StampPlaybackTarget | null {
	const sessionId = stamp.recording_session_id?.trim();
	if (
		sessionId &&
		stamp.session_started_at_ms != null &&
		Number.isFinite(stamp.session_started_at_ms)
	) {
		const relativeSeconds = Math.max(
			0,
			(stamp.stamp_ts - stamp.session_started_at_ms + stamp.offset_ms) / 1_000,
		);
		return {
			path: `/dashboard/${encodeURIComponent(guildId)}/audio/session/${encodeURIComponent(sessionId)}?t=${relativeSeconds}&clip=stamp`,
			relativeSeconds,
			scope: "session",
		};
	}

	if (
		!stamp.file_name ||
		stamp.start_ts == null ||
		stamp.year == null ||
		stamp.month == null ||
		stamp.month < 1 ||
		stamp.month > 12
	) {
		return null;
	}

	const relativeSeconds = Math.max(
		0,
		(stamp.stamp_ts - stamp.start_ts + stamp.offset_ms) / 1_000,
	);
	return {
		path: `/dashboard/${encodeURIComponent(guildId)}/audio/${encodeURIComponent(stamp.channel_id)}/${stamp.year}/${stamp.month}/${encodeURIComponent(stamp.file_name)}?t=${relativeSeconds}`,
		relativeSeconds,
		scope: "fragment",
	};
}
