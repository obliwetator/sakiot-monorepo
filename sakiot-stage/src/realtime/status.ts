import { useSyncExternalStore } from "react";

/**
 * - `off`: realtime is disabled on the server, or nobody is logged in.
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
 * otherwise `whenOffline` (today's interval for growing data, 30 s for the
 * rest so discovery still happens without realtime).
 */
export function useFallbackPolling(whenOffline: number): number {
	return useRealtimeLive() ? 0 : whenOffline;
}
