import { useEffect } from "react";
import { useRefreshMutation } from "./apiSlice";

const RECONNECT_INITIAL_DELAY_MS = 1_000;
const RECONNECT_MAX_DELAY_MS = 30_000;

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
	const [refreshToken] = useRefreshMutation();

	// biome-ignore lint/correctness/useExhaustiveDependencies: callers pass `deps` to control when the socket reconnects
	useEffect(() => {
		if (!enabled || !url) return;
		let disposed = false;
		let socket: WebSocket | null = null;
		let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
		let retryDelay = RECONNECT_INITIAL_DELAY_MS;

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
				retryDelay = RECONNECT_INITIAL_DELAY_MS;
				socket?.send(JSON.stringify(subscribe));
			};
			socket.onmessage = (event) => {
				try {
					onMessage(JSON.parse(event.data));
				} catch (e) {
					console.error("ws parse error", e);
				}
			};
			socket.onclose = (event) => {
				socket = null;
				if (disposed) return;
				if (event.code === 4001) {
					refreshToken()
						.unwrap()
						.then(() => {
							retryDelay = RECONNECT_INITIAL_DELAY_MS;
							onAuthRefreshed?.();
						})
						.catch(() => {
							// Session still not usable; the backoff schedule decides the retry.
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
	}, [enabled, url, refreshToken, ...(opts.deps ?? [])]);
}
