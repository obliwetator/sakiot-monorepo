import { useEffect } from "react";
import { ensureRefreshed } from "./authedFetch";

const RECONNECT_INITIAL_DELAY_MS = 1_000;
const RECONNECT_MAX_DELAY_MS = 30_000;

export function shouldAttemptWebSocketRefresh(
	closeCode: number,
	refreshedSinceLastMessage: boolean,
): boolean {
	return !refreshedSinceLastMessage && [1006, 1008, 4001].includes(closeCode);
}

export function useWebSocketStream(opts: {
	enabled: boolean;
	url: string | null;
	subscribe: unknown;
	unsubscribe: unknown;
	onMessage: (data: { event_type?: string; payload?: string }) => void;
	onAuthRefreshed?: () => void;
	deps?: unknown[];
}) {
	const { enabled, url, subscribe, unsubscribe, onMessage, onAuthRefreshed } =
		opts;

	// biome-ignore lint/correctness/useExhaustiveDependencies: callers pass `deps` to control when the socket reconnects
	useEffect(() => {
		if (!enabled || !url) return;
		let disposed = false;
		let socket: WebSocket | null = null;
		let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
		let retryDelay = RECONNECT_INITIAL_DELAY_MS;
		let refreshedSinceLastMessage = false;

		const scheduleReconnect = () => {
			if (disposed || reconnectTimer) return;
			reconnectTimer = setTimeout(() => {
				reconnectTimer = null;
				connect();
			}, retryDelay);
			retryDelay = Math.min(retryDelay * 2, RECONNECT_MAX_DELAY_MS);
		};

		const connect = () => {
			if (disposed) return;
			socket = new WebSocket(url);
			socket.onopen = () => {
				socket?.send(JSON.stringify(subscribe));
			};
			socket.onmessage = (event) => {
				// Receiving application data proves connection stability. Upgrade
				// alone does not: auth failure can close immediately afterwards.
				retryDelay = RECONNECT_INITIAL_DELAY_MS;
				refreshedSinceLastMessage = false;
				try {
					onMessage(JSON.parse(event.data));
				} catch (e) {
					console.error("ws parse error", e);
				}
			};
			socket.onclose = (event) => {
				socket = null;
				if (disposed) return;
				if (
					shouldAttemptWebSocketRefresh(event.code, refreshedSinceLastMessage)
				) {
					// Expired access cookies fail during HTTP upgrade, exposed by
					// browsers as abnormal close 1006. Shared refresh deduplicates
					// simultaneous global/guild socket failures.
					ensureRefreshed()
						.then((refreshed) => {
							if (!refreshed) return;
							refreshedSinceLastMessage = true;
							onAuthRefreshed?.();
						})
						.finally(() => scheduleReconnect());
				} else {
					scheduleReconnect();
				}
			};
		};

		connect();
		return () => {
			disposed = true;
			if (reconnectTimer) clearTimeout(reconnectTimer);
			if (socket && socket.readyState === WebSocket.OPEN)
				socket.send(JSON.stringify(unsubscribe));
			socket?.close();
		};
	}, [enabled, url, ...(opts.deps ?? [])]);
}
