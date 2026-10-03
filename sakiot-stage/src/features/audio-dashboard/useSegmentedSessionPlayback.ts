import { useEffect, useState, useSyncExternalStore } from "react";
import {
	SegmentedSessionEngine,
	type SegmentedSessionOptions,
} from "./segmentedSessionEngine";

/** React adapter for {@link SegmentedSessionEngine}: options in, snapshot out. */
export function useSegmentedSessionPlayback(options: SegmentedSessionOptions) {
	const [engine] = useState(() => new SegmentedSessionEngine(options));
	const snapshot = useSyncExternalStore(engine.subscribe, engine.getSnapshot);

	useEffect(() => {
		engine.setCallbacks(options.onError, options.onLoopDisabled);
	});
	useEffect(() => {
		engine.setTimeline(options.segments, options.durationMs);
	}, [engine, options.segments, options.durationMs]);
	useEffect(() => {
		engine.setPlaybackRate(options.playbackRate);
	}, [engine, options.playbackRate]);
	useEffect(() => {
		engine.setVolume(options.volume);
	}, [engine, options.volume]);
	useEffect(() => () => engine.dispose(), [engine]);

	return {
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
		restartWithSegments: engine.restartWithSegments,
	};
}

export type SegmentedSessionPlayback = ReturnType<
	typeof useSegmentedSessionPlayback
>;
