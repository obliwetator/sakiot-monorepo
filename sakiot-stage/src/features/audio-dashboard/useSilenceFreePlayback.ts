import { useCallback, useEffect, useRef, useState } from "react";
import {
	refreshForMediaRetry,
	SESSION_EXPIRED_MESSAGE,
} from "../../app/authedFetch";
import { clampPlaybackPosition } from "./logicalSessionPlaybackState";
import type { SessionSelection } from "./logicalSessionSelection";
import { usePlaybackBound } from "./usePlaybackBound";

interface SilencePlaybackOptions {
	mediaUrl: string | null;
	initialDurationMs?: number;
	volume: number;
	playbackRate: number;
	onLoopDisabled: () => void;
}

export function useSilenceFreePlayback(options: SilencePlaybackOptions) {
	const mediaUrl = options.mediaUrl;
	const [positionMs, setPositionMs] = useState(0);
	const [seekPreviewMs, setSeekPreviewMs] = useState<number | null>(null);
	const [durationMs, setDurationMs] = useState(0);
	const [playing, setPlaying] = useState(false);
	const [playbackError, setPlaybackError] = useState<string | null>(null);
	const [retryKey, setRetryKey] = useState(0);
	const audioRef = useRef<HTMLAudioElement | null>(null);
	const retryRef = useRef(false);
	const positionRef = useRef(0);
	const durationRef = useRef(options.initialDurationMs ?? 0);
	const playingRef = useRef(false);
	const startAtRef = useRef<(position: number, autoplay: boolean) => void>(
		() => {},
	);
	const onLoopDisabledRef = useRef(options.onLoopDisabled);
	const stopRef = useRef<() => void>(() => {});
	const onBeforePlayRef = useRef<() => void>(() => {});
	onBeforePlayRef.current = () => setPlaybackError(null);

	const bound = usePlaybackBound({
		startAtRef,
		positionRef,
		playingRef,
		durationRef,
		onLoopDisabledRef,
		stopRef,
		onBeforePlayRef,
		setSeekPreviewMs,
	});

	useEffect(() => {
		onLoopDisabledRef.current = options.onLoopDisabled;
	});
	useEffect(() => {
		positionRef.current = positionMs;
	}, [positionMs]);
	useEffect(() => {
		playingRef.current = playing;
	}, [playing]);
	useEffect(() => {
		if (audioRef.current) audioRef.current.volume = options.volume;
	}, [options.volume]);
	useEffect(() => {
		if (audioRef.current) audioRef.current.playbackRate = options.playbackRate;
	}, [options.playbackRate]);
	useEffect(() => {
		// Reading URL makes reset explicit: each generated media resource starts
		// with independent duration, position, and single-retry state.
		void mediaUrl;
		retryRef.current = false;
		durationRef.current = options.initialDurationMs ?? 0;
		positionRef.current = 0;
		setDurationMs(durationRef.current);
		setPositionMs(0);
		setSeekPreviewMs(null);
		setPlaybackError(null);
		bound.clearBound();
	}, [mediaUrl, options.initialDurationMs, bound.clearBound]);

	const stop = useCallback(() => {
		bound.clearBound();
		audioRef.current?.pause();
		playingRef.current = false;
		setPlaying(false);
	}, [bound.clearBound]);
	stopRef.current = stop;

	const startAt = useCallback(
		(requestedPosition: number, autoplay: boolean) => {
			const audio = audioRef.current;
			const position = clampPlaybackPosition(
				requestedPosition,
				durationRef.current,
			);
			positionRef.current = position;
			setPositionMs(position);
			setSeekPreviewMs(null);
			if (!audio) return;
			try {
				audio.currentTime = position / 1_000;
			} catch {
				setPlaybackError("Silence-free audio could not be seeked.");
				return;
			}
			if (!autoplay || position >= durationRef.current) {
				audio.pause();
				playingRef.current = false;
				setPlaying(false);
				return;
			}
			setPlaybackError(null);
			void audio.play().catch(() => {
				playingRef.current = false;
				setPlaying(false);
				bound.clearBound();
				setPlaybackError("Browser blocked or failed silence-free playback.");
			});
		},
		[bound.clearBound],
	);
	startAtRef.current = startAt;

	const seek = useCallback(
		(nextPositionMs: number, selection: SessionSelection, loop: boolean) => {
			setSeekPreviewMs(null);
			const target = clampPlaybackPosition(nextPositionMs, durationRef.current);
			bound.prepareSeek(selection, loop, target);
			const audio = audioRef.current;
			if (audio) {
				try {
					audio.currentTime = target / 1_000;
				} catch {
					setPlaybackError("Silence-free audio could not be seeked.");
					return;
				}
			}
			positionRef.current = target;
			setPositionMs(target);
			bound.applyBound(target);
		},
		[bound.applyBound, bound.prepareSeek],
	);

	const updateDuration = useCallback(
		(audio: HTMLAudioElement) => {
			if (!Number.isFinite(audio.duration) || audio.duration <= 0) return;
			const duration = audio.duration * 1_000;
			durationRef.current = duration;
			setDurationMs(duration);
			audio.volume = options.volume;
			audio.playbackRate = options.playbackRate;
		},
		[options.playbackRate, options.volume],
	);

	const onTimeUpdate = useCallback(
		(audio: HTMLAudioElement) => {
			const next = audio.currentTime * 1_000;
			if (bound.applyBound(next)) return;
			positionRef.current = next;
			setPositionMs(next);
		},
		[bound.applyBound],
	);

	const onError = useCallback(() => {
		playingRef.current = false;
		setPlaying(false);
		bound.clearBound();
		if (retryRef.current) {
			setPlaybackError("Silence-free audio could not be loaded.");
			return;
		}
		retryRef.current = true;
		void refreshForMediaRetry().then((ok) => {
			if (ok) {
				setPlaybackError(null);
				setRetryKey((key) => key + 1);
			} else {
				setPlaybackError(SESSION_EXPIRED_MESSAGE);
			}
		});
	}, [bound.clearBound]);

	useEffect(() => {
		if (!playing) return;
		let frame: number | null = null;
		const update = () => {
			const audio = audioRef.current;
			if (!audio || audio.paused) return;
			onTimeUpdate(audio);
			frame = requestAnimationFrame(update);
		};
		frame = requestAnimationFrame(update);
		return () => {
			if (frame !== null) cancelAnimationFrame(frame);
		};
	}, [onTimeUpdate, playing]);

	return {
		audioRef,
		positionMs,
		seekPreviewMs,
		setSeekPreviewMs,
		durationMs,
		playing,
		playbackError,
		retryKey,
		boundActive: bound.boundActive,
		startAt,
		seek,
		togglePlay: bound.togglePlay,
		togglePreview: bound.togglePreview,
		updateLoop: bound.updateLoop,
		syncBound: bound.syncBound,
		clearBound: bound.clearBound,
		stop,
		mediaHandlers: {
			onLoadedMetadata: (audio: HTMLAudioElement) => {
				retryRef.current = false;
				updateDuration(audio);
			},
			onDurationChange: updateDuration,
			onTimeUpdate,
			onPlay: () => {
				playingRef.current = true;
				setPlaying(true);
			},
			onPause: () => {
				playingRef.current = false;
				setPlaying(false);
			},
			onEnded: () => {
				playingRef.current = false;
				setPlaying(false);
				bound.clearBound();
			},
			onError,
		},
	};
}

export type SilenceFreePlayback = ReturnType<typeof useSilenceFreePlayback>;
