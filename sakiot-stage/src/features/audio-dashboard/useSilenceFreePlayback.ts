import { useCallback, useEffect, useRef, useState } from "react";
import {
	refreshForMediaRetry,
	SESSION_EXPIRED_MESSAGE,
} from "../../app/authedFetch";
import { clampPlaybackPosition } from "./logicalSessionPlaybackState";
import type { SessionSelection } from "./logicalSessionSelection";
import { selectionContainsPosition } from "./logicalSessionSelection";

interface PlaybackBound {
	stopMs: number;
	loopToMs: number | null;
}

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
	const [boundActive, setBoundActive] = useState(false);
	const audioRef = useRef<HTMLAudioElement | null>(null);
	const retryRef = useRef(false);
	const positionRef = useRef(0);
	const durationRef = useRef(options.initialDurationMs ?? 0);
	const playingRef = useRef(false);
	const boundRef = useRef<PlaybackBound | null>(null);
	const startAtRef = useRef<(position: number, autoplay: boolean) => void>(
		() => {},
	);
	const onLoopDisabledRef = useRef(options.onLoopDisabled);

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
		boundRef.current = null;
		setBoundActive(false);
	}, [mediaUrl, options.initialDurationMs]);

	const clearBound = useCallback(() => {
		boundRef.current = null;
		setBoundActive(false);
	}, []);

	const stop = useCallback(() => {
		clearBound();
		audioRef.current?.pause();
		playingRef.current = false;
		setPlaying(false);
	}, [clearBound]);

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
				clearBound();
				setPlaybackError("Browser blocked or failed silence-free playback.");
			});
		},
		[clearBound],
	);
	startAtRef.current = startAt;

	const applyBound = useCallback((atMs: number): boolean => {
		const bound = boundRef.current;
		if (!bound || atMs < bound.stopMs) return false;
		if (bound.loopToMs !== null) {
			startAtRef.current(bound.loopToMs, true);
			return true;
		}
		boundRef.current = null;
		setBoundActive(false);
		startAtRef.current(bound.stopMs, false);
		return true;
	}, []);

	const seek = useCallback(
		(nextPositionMs: number, selection: SessionSelection, loop: boolean) => {
			setSeekPreviewMs(null);
			const target = clampPlaybackPosition(nextPositionMs, durationRef.current);
			if (loop && !selectionContainsPosition(selection, target)) {
				clearBound();
				onLoopDisabledRef.current();
			} else if (boundRef.current) {
				boundRef.current = {
					stopMs: selection[1],
					loopToMs: loop ? selection[0] : null,
				};
			}
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
			applyBound(target);
		},
		[applyBound, clearBound],
	);

	const togglePlay = useCallback(
		(selection: SessionSelection, loop: boolean) => {
			if (playingRef.current) {
				stop();
				return;
			}
			setPlaybackError(null);
			if (loop && selection[1] > selection[0]) {
				boundRef.current = { stopMs: selection[1], loopToMs: selection[0] };
				setBoundActive(true);
				startAtRef.current(
					selectionContainsPosition(selection, positionRef.current)
						? positionRef.current
						: selection[0],
					true,
				);
				return;
			}
			clearBound();
			startAtRef.current(
				positionRef.current >= durationRef.current ? 0 : positionRef.current,
				true,
			);
		},
		[clearBound, stop],
	);

	const togglePreview = useCallback(
		(selection: SessionSelection, loop: boolean) => {
			if (boundRef.current) {
				stop();
				return;
			}
			if (selection[1] <= selection[0]) return;
			boundRef.current = {
				stopMs: selection[1],
				loopToMs: loop ? selection[0] : null,
			};
			setBoundActive(true);
			setSeekPreviewMs(null);
			startAtRef.current(selection[0], true);
		},
		[stop],
	);

	const updateLoop = useCallback(
		(enabled: boolean, selection: SessionSelection) => {
			if (!enabled) {
				if (boundRef.current) boundRef.current.loopToMs = null;
				return;
			}
			if (boundRef.current) {
				boundRef.current = { stopMs: selection[1], loopToMs: selection[0] };
				setBoundActive(true);
				startAtRef.current(selection[0], playingRef.current);
				return;
			}
			clearBound();
			startAtRef.current(selection[0], false);
		},
		[clearBound],
	);

	const syncBound = useCallback(
		(selection: SessionSelection, loop: boolean) => {
			if (!boundRef.current) return;
			if (!selectionContainsPosition(selection, positionRef.current)) {
				clearBound();
				if (loop) onLoopDisabledRef.current();
				return;
			}
			boundRef.current = {
				stopMs: selection[1],
				loopToMs: loop ? selection[0] : null,
			};
			applyBound(positionRef.current);
		},
		[applyBound, clearBound],
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
			if (applyBound(next)) return;
			positionRef.current = next;
			setPositionMs(next);
		},
		[applyBound],
	);

	const onError = useCallback(() => {
		playingRef.current = false;
		setPlaying(false);
		clearBound();
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
	}, [clearBound]);

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
		boundActive,
		startAt,
		seek,
		togglePlay,
		togglePreview,
		updateLoop,
		syncBound,
		clearBound,
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
				clearBound();
			},
			onError,
		},
	};
}

export type SilenceFreePlayback = ReturnType<typeof useSilenceFreePlayback>;
