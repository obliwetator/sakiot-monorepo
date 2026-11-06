/** Consecutive failed polls tolerated before the waveform poller gives up. */
export const CLIP_WAVEFORM_MAX_POLL_ERRORS = 5;

/**
 * Whether a request not yet counted has finished. This goes by its id rather
 * than by watching `isFetching` fall: RTK Query tells subscribers about a
 * request on the next animation frame, so a response that beats it, as a
 * network failing instantly does, never shows as in flight.
 */
export function pollSettled(
	requestId: string | undefined,
	countedRequestId: string | undefined,
	isFetching: boolean,
): boolean {
	return !isFetching && requestId !== countedRequestId;
}

/**
 * The failure streak after a poll settles. Only completed requests count: a
 * refetch clears RTK Query's `isError` while it is in flight, so a counter
 * driven by `isError` alone would reset on every poll and never reach the cap.
 */
export function nextPollErrorCount(
	settled: boolean,
	failed: boolean,
	current: number,
): number {
	if (!settled) return current;
	return failed ? current + 1 : 0;
}

/**
 * Whether the clip-waveform poller should keep running.
 *
 * The waveform is built asynchronously after the clip is stored, and a poll
 * can fail transiently (server restart, dropped request) while that build is
 * still in flight. The poller therefore keeps going until the server reports
 * 100%, retrying transport errors; a server-reported build error is terminal,
 * and a run of consecutive failures gives up and shows the manual retry button.
 */
export function shouldKeepPollingClipWaveform(
	progress: number | undefined,
	consecutiveErrors: number,
	serverError = false,
): boolean {
	if (serverError) return false;
	if (progress === 100) return false;
	return consecutiveErrors < CLIP_WAVEFORM_MAX_POLL_ERRORS;
}
