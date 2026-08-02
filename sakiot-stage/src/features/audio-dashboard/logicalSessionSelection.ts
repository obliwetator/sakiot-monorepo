export type SessionSelection = [number, number];

export const MIN_CLIP_DURATION_MS = 1_000;
export const MAX_CLIP_DURATION_MS = 20_000;
export const DEFAULT_CLIP_DURATION_MS = 15_000;
export const STAMP_DRAFT_LEAD_MS = 10_000;

export interface SelectionManifest {
	recordingSessionId: string;
	durationMs: number;
}

/** A new session starts with something that can immediately become a clip. */
export function defaultClipSelection(durationMs: number): SessionSelection {
	const duration = Math.max(0, durationMs);
	return [0, Math.min(DEFAULT_CLIP_DURATION_MS, duration)];
}

export function reconcileSessionSelection(
	selection: SessionSelection,
	previousManifest: SelectionManifest | null,
	nextManifest: SelectionManifest,
): SessionSelection {
	const durationMs = Math.max(0, nextManifest.durationMs);
	if (
		previousManifest === null ||
		previousManifest.recordingSessionId !== nextManifest.recordingSessionId
	) {
		return defaultClipSelection(durationMs);
	}

	const previousDefault = defaultClipSelection(previousManifest.durationMs);
	const followedPreviousDefault =
		selection[0] === previousDefault[0] && selection[1] === previousDefault[1];
	const followedPreviousMax = selection[1] === previousManifest.durationMs;
	if (
		selection[1] === 0 ||
		selection[1] > durationMs ||
		followedPreviousDefault
	) {
		return defaultClipSelection(durationMs);
	}
	if (followedPreviousMax) {
		return [Math.min(selection[0], durationMs), durationMs];
	}

	return selection;
}

export function isValidClipSelection(selection: SessionSelection): boolean {
	const durationMs = selection[1] - selection[0];
	return (
		durationMs >= MIN_CLIP_DURATION_MS && durationMs <= MAX_CLIP_DURATION_MS
	);
}

/**
 * Starts a clip draft around the moment that prompted a stamp. Most of the
 * context is placed before the stamp because people usually stamp just after
 * hearing something worth keeping. Near either session boundary the complete
 * draft slides inward instead of becoming shorter.
 */
export function selectionAroundStamp(
	stampMs: number,
	durationMs: number,
): SessionSelection {
	const duration = Math.max(0, durationMs);
	const length = Math.min(DEFAULT_CLIP_DURATION_MS, duration);
	const latestStart = Math.max(0, duration - length);
	const preferredStart = stampMs - STAMP_DRAFT_LEAD_MS;
	const start = Math.min(Math.max(preferredStart, 0), latestStart);
	return [start, start + length];
}

/** Restores the initial draft while retaining stamp-relative context. */
export function resetClipSelection(
	durationMs: number,
	stampMs?: number,
): SessionSelection {
	return stampMs === undefined
		? defaultClipSelection(durationMs)
		: selectionAroundStamp(stampMs, durationMs);
}
