import type { SessionManifest, SessionSegment } from "../../app/apiSlice";

export type PlaybackSegment = SessionSegment & {
	start_ms: number;
	end_ms: number;
};

/**
 * Produces a complete, non-overlapping logical timeline. Server-provided gaps
 * win in timestamp order; uncovered regions become synthetic silence so seek
 * and playback remain continuous across fragments and an active tail.
 */
export function normalizeSessionSegments(
	manifest: SessionManifest,
): PlaybackSegment[] {
	const sorted = manifest.segments
		.filter((segment) => segment.end_ms > segment.start_ms)
		.slice()
		.sort((left, right) => left.start_ms - right.start_ms);
	const normalized: PlaybackSegment[] = [];
	let cursor = 0;
	for (const segment of sorted) {
		if (segment.start_ms > cursor) {
			normalized.push({
				kind: "silence",
				start_ms: cursor,
				end_ms: segment.start_ms,
				reason: "implicit_silence",
			});
		}
		const start = Math.max(cursor, segment.start_ms);
		if (segment.end_ms > start) {
			normalized.push({ ...segment, start_ms: start });
			cursor = segment.end_ms;
		}
	}
	if (cursor < manifest.duration_ms) {
		normalized.push({
			kind: "silence",
			start_ms: cursor,
			end_ms: manifest.duration_ms,
			reason: "active_silence",
		});
	}
	return normalized;
}
