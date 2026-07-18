export type SessionSelection = [number, number];

export const MIN_CLIP_DURATION_MS = 1_000;
export const MAX_CLIP_DURATION_MS = 20_000;

export interface SelectionManifest {
	recordingSessionId: string;
	durationMs: number;
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
		return [0, durationMs];
	}

	const followedPreviousMax = selection[1] === previousManifest.durationMs;
	if (selection[1] === 0 || selection[1] > durationMs || followedPreviousMax) {
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
