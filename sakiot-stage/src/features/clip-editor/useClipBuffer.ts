import { useEffect, useState } from "react";
import { API_ROUTES, apiUrl } from "../../api/routes";
import {
	ApiRequestError,
	problemFromError,
	problemFromResponse,
} from "../../app/apiError";
import { authedFetch } from "../../app/authedFetch";

import { PcmBudget, SOURCE_CACHE_BYTES } from "./pcmBudget";

const cacheBudget = new PcmBudget<string>(SOURCE_CACHE_BYTES);
let decodeQueue: Promise<unknown> = Promise.resolve();
const bufferCache = new Map<string, Promise<AudioBuffer>>();
let decodeContext: AudioContext | null = null;
const SHARED_DSP_SAMPLE_RATE = 48_000;
const LOAD_FAILED = "Clip audio could not be loaded.";

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

function evictBuffer(key: string, promise: Promise<AudioBuffer>): void {
	if (bufferCache.get(key) !== promise) return;
	bufferCache.delete(key);
}

export function loadClipBuffer(
	guildId: string,
	clipId: string,
): Promise<AudioBuffer> {
	const key = clipBufferKey(guildId, clipId);
	const cached = bufferCache.get(key);
	if (cached) {
		cacheBudget.touch(key);
		return cached;
	}
	const promise = decodeQueue.then(async () => {
		const response = await authedFetch(
			apiUrl(API_ROUTES.clip, { guild_id: guildId, clip_id: clipId }),
		);
		if (!response.ok) {
			// authedFetch already retried once after a refresh, so a 401 here
			// means the session is over; problemFromResponse words it so.
			const problem = await problemFromResponse(response);
			throw new ApiRequestError(
				response.status === 403
					? {
							...problem,
							message: "You don't have access to this clip's channel.",
						}
					: problem,
			);
		}
		const bytes = await response.arrayBuffer();
		try {
			return await contextForDecoding().decodeAudioData(bytes);
		} catch {
			throw new ApiRequestError({
				cause: "malformed",
				status: response.status,
				kind: null,
				message:
					"The clip audio could not be decoded. The file may be damaged or in an unsupported format.",
			});
		}
	});
	decodeQueue = promise.then(
		() => undefined,
		() => undefined,
	);
	bufferCache.set(key, promise);
	promise.then(
		(buffer) => {
			cacheBudget.retain(key, buffer.length * buffer.numberOfChannels * 4, () =>
				evictBuffer(key, promise),
			);
		},
		() => {
			if (bufferCache.get(key) === promise) bufferCache.delete(key);
		},
	);
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
	useEffect(() => {
		if (!clipId) {
			setState({ status: "idle", buffer: null, error: null });
			return;
		}
		let active = true;
		setState({ status: "loading", buffer: null, error: null });
		loadClipBuffer(guildId, clipId)
			.then((buffer) => {
				if (active) {
					setState({ status: "ready", buffer, error: null });
				}
			})
			.catch((error: unknown) => {
				if (active) {
					setState({
						status: "error",
						buffer: null,
						error: `${LOAD_FAILED} ${problemFromError(error).message}`,
					});
				}
			});
		return () => {
			// The cached load can serve other consumers; only stop this effect
			// from publishing after its source changes or React replays it.
			active = false;
		};
	}, [clipId, guildId]);

	return state;
}
