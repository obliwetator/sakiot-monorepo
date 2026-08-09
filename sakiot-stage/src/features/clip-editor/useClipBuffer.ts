import { useEffect, useRef, useState } from "react";
import { authedFetch, SESSION_EXPIRED_MESSAGE } from "../../app/authedFetch";

const bufferCache = new Map<string, Promise<AudioBuffer>>();
let decodeContext: AudioContext | null = null;
const SHARED_DSP_SAMPLE_RATE = 48_000;

function contextForDecoding(): AudioContext {
	if (!decodeContext) {
		// Keep source frame boundaries and DSP coefficients aligned with the
		// server's canonical FFmpeg decode rate on every playback device.
		decodeContext = new AudioContext({ sampleRate: SHARED_DSP_SAMPLE_RATE });
	}
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
			// authedFetch already retried once after a refresh; a 401 here
			// means the refresh token is gone too.
			if (response.status === 401) throw new Error(SESSION_EXPIRED_MESSAGE);
			if (response.status === 403) {
				throw new Error("You don't have access to this clip's channel.");
			}
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
	error: string | null;
};

export function useClipBuffer(
	guildId: string,
	clipId: string | null,
): ClipBufferState {
	const [state, setState] = useState<ClipBufferState>({
		status: "idle",
		buffer: null,
		error: null,
	});
	const requestRef = useRef(0);

	useEffect(() => {
		if (!clipId) {
			setState({ status: "idle", buffer: null, error: null });
			return;
		}
		const request = ++requestRef.current;
		setState({ status: "loading", buffer: null, error: null });
		loadClipBuffer(guildId, clipId)
			.then((buffer) => {
				if (requestRef.current === request) {
					setState({ status: "ready", buffer, error: null });
				}
			})
			.catch((error: unknown) => {
				if (requestRef.current === request) {
					setState({
						status: "error",
						buffer: null,
						error:
							error instanceof Error
								? error.message
								: "Clip audio could not be loaded.",
					});
				}
			});
	}, [clipId, guildId]);

	return state;
}
