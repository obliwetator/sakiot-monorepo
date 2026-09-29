import { useEffect, useState, useSyncExternalStore } from "react";
import { ChannelMixEngine, type ChannelMixOptions } from "./channelMixEngine";

/** React adapter for {@link ChannelMixEngine}: options in, snapshot out. */
export function useChannelMixPlayback(options: ChannelMixOptions) {
	const [engine] = useState(() => new ChannelMixEngine(options));
	const snapshot = useSyncExternalStore(engine.subscribe, engine.getSnapshot);

	useEffect(() => {
		engine.setSourceErrorListener(options.onSourceError);
	}, [engine, options.onSourceError]);
	useEffect(() => {
		engine.setTracks(options.tracks);
	}, [engine, options.tracks]);
	useEffect(() => {
		engine.setDurationMs(options.durationMs);
	}, [engine, options.durationMs]);
	useEffect(() => {
		engine.setSettings(options.settings);
	}, [engine, options.settings]);
	useEffect(() => {
		engine.setVolume(options.volume);
	}, [engine, options.volume]);
	useEffect(() => {
		engine.setPlaybackRate(options.playbackRate);
	}, [engine, options.playbackRate]);
	useEffect(() => () => engine.dispose(), [engine]);

	return {
		...snapshot,
		seek: engine.seek,
		togglePlay: engine.togglePlay,
		goLive: engine.goLive,
		stop: engine.pause,
	};
}
