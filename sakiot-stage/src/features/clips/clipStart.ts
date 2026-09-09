import type { ClipData } from "../../app/apiSlice";

/**
 * Epoch milliseconds encoded in a recording stem (`{epoch_ms}-{user_id}`).
 * Anything else — a composed clip, a session-derived clip, a renamed source —
 * has no timestamp of its own and returns null.
 */
export function recordingStartFromStem(
	originalFileName: string | null | undefined,
): number | null {
	if (!originalFileName) return null;
	const [prefix] = originalFileName.split("-");
	if (!prefix || !/^\d+$/.test(prefix)) return null;
	const timestamp = Number(prefix);
	return Number.isSafeInteger(timestamp) ? timestamp : null;
}

/**
 * Absolute start of a clip in epoch milliseconds.
 *
 * Session-derived clips carry `session:{id}` in `original_file_name`, so the
 * timestamp in that stem says nothing; their base time comes from the session
 * manifest. Single-recording clips keep the timestamp in their source stem.
 */
export function clipAbsoluteStartMs(
	clip: ClipData | null,
	sessionStartedAtMs: number | null,
): number | null {
	if (!clip) return null;
	const base =
		sessionStartedAtMs ?? recordingStartFromStem(clip.original_file_name);
	if (base === null || !Number.isFinite(clip.start_time)) return null;
	return base + clip.start_time * 1000;
}
