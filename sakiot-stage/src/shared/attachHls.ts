import type Hls from "hls.js";

/** Result of an `attachHlsAudio` attempt. */
export type HlsAttachResult =
	| { kind: "native"; hls: null }
	| { kind: "hls"; hls: Hls }
	| { kind: "direct"; hls: null }
	| { kind: "unsupported"; hls: null }
	| { kind: "inactive"; hls: null };

/**
 * Safari natively decodes HLS and returns "probably" — skip the hls.js
 * download. Some Chromium builds return "maybe" without actually being able
 * to decode HLS, which lands in the native path and immediately fails;
 * require "probably" so those browsers fall through to hls.js.
 */
export function prefersNativeHls(audio: HTMLAudioElement): boolean {
	return audio.canPlayType("application/vnd.apple.mpegurl") === "probably";
}

/**
 * Attaches an HLS playlist to an audio element, choosing between native
 * playback (Safari) and the code-split hls.js loader. Shared by the three
 * playback engines; per-engine behavior is expressed through the options:
 *
 * - `fallbackUrl` — direct media URL used when neither native HLS nor hls.js
 *   can serve the playlist (result `kind: "direct"`).
 * - `onFatal` — invoked for fatal hls.js errors and for the
 *   unsupported/failed-import cases when no fallback URL is given.
 * - `onManifestParsed` — the hls.js start signal; the native path relies on
 *   the caller's own `loadedmetadata`/`canplay` listeners.
 * - `unlimitedMaxLatency` — disables hls.js's max-live-latency enforcement
 *   for callers that manage drift themselves (segmented session and the
 *   live AudioInterface); channel mix keeps hls.js defaults.
 * - `isActive` — re-checked after the dynamic import resolves so a cancelled
 *   caller never receives a leaked `Hls` instance.
 *
 * The caller owns the returned instance and must `destroy()` it on cleanup.
 */
export async function attachHlsAudio(options: {
	audio: HTMLAudioElement;
	playlistUrl: string;
	fallbackUrl?: string;
	isActive?: () => boolean;
	onFatal?: (message: string) => void;
	onManifestParsed?: () => void;
	unlimitedMaxLatency?: boolean;
}): Promise<HlsAttachResult> {
	const { audio, playlistUrl } = options;
	if (prefersNativeHls(audio)) {
		audio.src = playlistUrl;
		return { kind: "native", hls: null };
	}

	let HlsClass: typeof Hls;
	try {
		// Code-split so hls.js never lands in the initial bundle; it is
		// loaded only when an HLS-eligible recording is opened.
		({ default: HlsClass } = await import("hls.js"));
	} catch (error) {
		options.onFatal?.(`hls.js import failed: ${error}`);
		return { kind: "unsupported", hls: null };
	}
	if (options.isActive?.() === false) {
		return { kind: "inactive", hls: null };
	}
	if (!HlsClass.isSupported()) {
		if (options.fallbackUrl !== undefined) {
			audio.src = options.fallbackUrl;
			return { kind: "direct", hls: null };
		}
		options.onFatal?.("hls not supported in this browser");
		return { kind: "unsupported", hls: null };
	}

	const hls = new HlsClass({
		xhrSetup: (request) => {
			request.withCredentials = true;
		},
		liveSyncDuration: 2,
		...(options.unlimitedMaxLatency
			? { liveMaxLatencyDuration: Number.MAX_SAFE_INTEGER }
			: {}),
	});
	if (options.onManifestParsed) {
		hls.on(HlsClass.Events.MANIFEST_PARSED, options.onManifestParsed);
	}
	hls.on(HlsClass.Events.ERROR, (_event, data) => {
		if (data.fatal) {
			options.onFatal?.(`hls fatal: ${data.type}/${data.details}`);
		}
	});
	hls.loadSource(playlistUrl);
	hls.attachMedia(audio);
	return { kind: "hls", hls };
}
