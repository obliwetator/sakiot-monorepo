import { useEffect, useState, useSyncExternalStore } from "react";
import {
	SilenceFreeEngine,
	type SilenceFreeOptions,
} from "./silenceFreeEngine";

/** React adapter for {@link SilenceFreeEngine}: options in, snapshot out. */
export function useSilenceFreePlayback(options: SilenceFreeOptions) {
	const [engine] = useState(() => new SilenceFreeEngine(options));
	const snapshot = useSyncExternalStore(engine.subscribe, engine.getSnapshot);

	useEffect(() => {
		engine.setLoopDisabledListener(options.onLoopDisabled);
	});
	useEffect(() => {
		engine.setMedia(options.mediaUrl, options.initialDurationMs);
	}, [engine, options.mediaUrl, options.initialDurationMs]);
	useEffect(() => {
		engine.setVolume(options.volume);
	}, [engine, options.volume]);
	useEffect(() => {
		engine.setPlaybackRate(options.playbackRate);
	}, [engine, options.playbackRate]);
	useEffect(() => () => engine.dispose(), [engine]);

	return {
		audioRef: engine.audioRef,
		...snapshot,
		setSeekPreviewMs: engine.setSeekPreviewMs,
		startAt: engine.startAt,
		seek: engine.seek,
		togglePlay: engine.togglePlay,
		togglePreview: engine.togglePreview,
		updateLoop: engine.updateLoop,
		syncBound: engine.syncBound,
		clearBound: engine.clearBound,
		stop: engine.stop,
		mediaHandlers: engine.mediaHandlers,
	};
}

export type SilenceFreePlayback = ReturnType<typeof useSilenceFreePlayback>;
