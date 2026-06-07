import { useEffect } from "react";
import { useRefreshMutation } from "./apiSlice";

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
		const ws = new WebSocket(url);
		ws.onopen = () => ws.send(JSON.stringify(subscribe));
		ws.onmessage = (event) => {
			try {
				onMessage(JSON.parse(event.data));
			} catch (e) {
				console.error("ws parse error", e);
			}
		};
		ws.onclose = async (event) => {
			if (event.code === 4001) {
				try {
					await refreshToken().unwrap();
					onAuthRefreshed?.();
				} catch {
					/* ignored */
				}
			}
		};
		return () => {
			if (ws.readyState === WebSocket.OPEN)
				ws.send(JSON.stringify(unsubscribe));
			ws.close();
		};
	}, [enabled, url, refreshToken, ...(opts.deps ?? [])]);
}
