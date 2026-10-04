import { useSyncExternalStore } from "react";

/**
 * - `off`: nobody is logged in.
 * - `connecting`: opening the socket or (re)subscribing to the current scope.
 * - `live`: subscribed to the current scope; polling can stop.
 * - `fallback`: the socket keeps failing while HTTP works (a proxy without
 *   WebSocket support, a rejected origin); poll and retry slowly.
 * - `unsupported`: the server speaks a newer protocol; reload to update.
 */
export type RealtimeStatus =
	| "off"
	| "connecting"
	| "live"
	| "fallback"
	| "unsupported";

let status: RealtimeStatus = "off";
const listeners = new Set<() => void>();

export function getRealtimeStatus(): RealtimeStatus {
	return status;
}

export function setRealtimeStatus(next: RealtimeStatus): void {
	if (next === status) return;
	status = next;
	for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

export function useRealtimeStatus(): RealtimeStatus {
	return useSyncExternalStore(subscribe, getRealtimeStatus, getRealtimeStatus);
}

/** True while realtime delivers changes for the current page: skip polling. */
export function useRealtimeLive(): boolean {
	return useRealtimeStatus() === "live";
}

/**
 * Polling interval for a resource that realtime keeps fresh: none while live,
 * otherwise `whenOffline` (the interval for growing data, 30 s for the rest
 * so discovery still happens without realtime).
 */
export function useFallbackPolling(whenOffline: number): number {
	return useRealtimeLive() ? 0 : whenOffline;
}

/** While live, how often a page waiting on a job still checks it. */
export const JOB_POLL_WHILE_LIVE_MS = 5_000;

/**
 * Polling interval for a job's progress (`interval`, or 0 when not waiting).
 * While realtime is live, `jobs` events refresh the job as it changes, and a
 * slow poll only covers an event that arrives before the page knows the job's
 * id.
 */
export function jobPollingInterval(interval: number, live: boolean): number {
	return interval > 0 && live
		? Math.max(interval, JOB_POLL_WHILE_LIVE_MS)
		: interval;
}

export function useJobPolling(interval: number): number {
	return jobPollingInterval(interval, useRealtimeLive());
}
