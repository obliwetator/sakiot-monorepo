import type { SessionSelection } from "./logicalSessionSelection";
import type { PlaybackSegment } from "./logicalSessionTimeline";

export interface SessionDeepLink {
	positionMs: number;
	fromStamp: boolean;
	silenceFree: boolean;
}

export function parseSessionDeepLink(search: string): SessionDeepLink | null {
	const params = new URLSearchParams(search);
	const rawPosition = params.get("t");
	if (rawPosition === null) return null;
	const seconds = Number(rawPosition);
	if (!Number.isFinite(seconds) || seconds < 0) return null;
	return {
		positionMs: seconds * 1_000,
		fromStamp: params.get("clip") === "stamp",
		silenceFree: params.get("timeline") === "silence-free",
	};
}

export function segmentAtPosition(
	segments: readonly PlaybackSegment[],
	positionMs: number,
): PlaybackSegment | undefined {
	return segments.find(
		(segment) => positionMs >= segment.start_ms && positionMs < segment.end_ms,
	);
}

export function clampPlaybackPosition(
	positionMs: number,
	durationMs: number,
): number {
	return Math.max(0, Math.min(positionMs, durationMs));
}

export function shouldRetryMediaLoad(
	retryAttempted: boolean,
	generationMatches: boolean,
): boolean {
	return generationMatches && !retryAttempted;
}

export function selectionForTab(
	tab: "normal" | "silence",
	normal: SessionSelection,
	silence: SessionSelection | null,
	silenceDurationMs: number,
): SessionSelection {
	if (tab === "normal") return normal;
	return silence ?? [0, Math.max(0, silenceDurationMs)];
}

export function isSameMediaSegment(
	left: PlaybackSegment | null,
	right: PlaybackSegment | undefined,
): boolean {
	if (!left || !right || left.kind === "silence" || right.kind === "silence") {
		return false;
	}
	if (left.kind !== right.kind || left.start_ms !== right.start_ms)
		return false;
	if (left.audio_file_id && right.audio_file_id) {
		return left.audio_file_id === right.audio_file_id;
	}
	return Boolean(left.media_url && left.media_url === right.media_url);
}
