import { useEffect, useRef, useState } from "react";
import { authedFetch } from "../../app/authedFetch";

const bufferCache = new Map<string, Promise<AudioBuffer>>();
let decodeContext: AudioContext | null = null;

function contextForDecoding(): AudioContext {
	if (!decodeContext) decodeContext = new AudioContext();
	return decodeContext;
}

export function clipBufferKey(guildId: string, clipId: string): string {
	return `${guildId}/${clipId}`;
}

export function loadClipBuffer(
	guildId: string,
	clipId: string,
): Promise<AudioBuffer> {
	const key = clipBufferKey(guildId, clipId);
	const cached = bufferCache.get(key);
	if (cached) return cached;
	const promise = (async () => {
		const response = await authedFetch(
			`audio/clips/${guildId}/${encodeURIComponent(clipId)}`,
		);
		if (!response.ok) {
			throw new Error(`Clip audio load failed (${response.status}).`);
		}
		const bytes = await response.arrayBuffer();
		return contextForDecoding().decodeAudioData(bytes);
	})();
	bufferCache.set(key, promise);
	promise.catch(() => bufferCache.delete(key));
	return promise;
}

export type ClipBufferState = {
	status: "idle" | "loading" | "ready" | "error";
	buffer: AudioBuffer | null;
};

export function useClipBuffer(
	guildId: string,
	clipId: string | null,
): ClipBufferState {
	const [state, setState] = useState<ClipBufferState>({
		status: "idle",
		buffer: null,
	});
	const requestRef = useRef(0);

	useEffect(() => {
		if (!clipId) {
			setState({ status: "idle", buffer: null });
			return;
		}
		const request = ++requestRef.current;
		setState({ status: "loading", buffer: null });
		loadClipBuffer(guildId, clipId)
			.then((buffer) => {
				if (requestRef.current === request) {
					setState({ status: "ready", buffer });
				}
			})
			.catch(() => {
				if (requestRef.current === request) {
					setState({ status: "error", buffer: null });
				}
			});
	}, [clipId, guildId]);

	return state;
}
