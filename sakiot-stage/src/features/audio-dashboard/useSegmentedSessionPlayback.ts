import type Hls from "hls.js";
import { useCallback, useEffect, useRef, useState } from "react";
import { BASE_API_URL } from "../../app/apiSlice";
import {
	refreshForMediaRetry,
	SESSION_EXPIRED_MESSAGE,
} from "../../app/authedFetch";
import {
	clampPlaybackPosition,
	isSameMediaSegment,
	segmentAtPosition,
	shouldRetryMediaLoad,
} from "./logicalSessionPlaybackState";
import type { SessionSelection } from "./logicalSessionSelection";
import { selectionContainsPosition } from "./logicalSessionSelection";
import type { PlaybackSegment } from "./logicalSessionTimeline";

interface PlaybackBound {
	stopMs: number;
	loopToMs: number | null;
}

interface SegmentedPlaybackOptions {
	segments: PlaybackSegment[];
	durationMs: number;
	volume: number;
	playbackRate: number;
	onError: (message: string | null) => void;
	onLoopDisabled: () => void;
}

function absoluteMediaUrl(path: string): string {
	return new URL(
		path,
		new URL(BASE_API_URL, window.location.origin),
	).toString();
}

export function useSegmentedSessionPlayback(options: SegmentedPlaybackOptions) {
	const [positionMs, setPositionMs] = useState(0);
	const [seekPreviewMs, setSeekPreviewMs] = useState<number | null>(null);
	const [playing, setPlaying] = useState(false);
	const [boundActive, setBoundActive] = useState(false);
	const audioRef = useRef<HTMLAudioElement | null>(null);
	const hlsRef = useRef<Hls | null>(null);
	const animationRef = useRef<number | null>(null);
	const generationRef = useRef(0);
	const mediaRetryRef = useRef(false);
	const positionRef = useRef(0);
	const playingRef = useRef(false);
	const activeSegmentRef = useRef<PlaybackSegment | null>(null);
	const durationRef = useRef(options.durationMs);
	const segmentsRef = useRef(options.segments);
	const rateRef = useRef(options.playbackRate);
	const volumeRef = useRef(options.volume);
	const boundRef = useRef<PlaybackBound | null>(null);
	const startAtRef = useRef<(position: number, autoplay: boolean) => void>(
		() => {},
	);
	const onErrorRef = useRef(options.onError);
	const onLoopDisabledRef = useRef(options.onLoopDisabled);

	useEffect(() => {
		onErrorRef.current = options.onError;
		onLoopDisabledRef.current = options.onLoopDisabled;
	});
	useEffect(() => {
		positionRef.current = positionMs;
	}, [positionMs]);
	useEffect(() => {
		playingRef.current = playing;
	}, [playing]);
	useEffect(() => {
		segmentsRef.current = options.segments;
	}, [options.segments]);
	useEffect(() => {
		durationRef.current = options.durationMs;
		setPositionMs((current) => Math.min(current, options.durationMs));
		setSeekPreviewMs((current) =>
			current === null ? null : Math.min(current, options.durationMs),
		);
	}, [options.durationMs]);
	useEffect(() => {
		rateRef.current = options.playbackRate;
		if (audioRef.current) audioRef.current.playbackRate = options.playbackRate;
	}, [options.playbackRate]);
	useEffect(() => {
		volumeRef.current = options.volume;
		if (audioRef.current) audioRef.current.volume = options.volume;
	}, [options.volume]);

	const stopSource = useCallback(() => {
		if (animationRef.current !== null) {
			cancelAnimationFrame(animationRef.current);
			animationRef.current = null;
		}
		hlsRef.current?.destroy();
		hlsRef.current = null;
		if (audioRef.current) {
			audioRef.current.pause();
			audioRef.current.removeAttribute("src");
			audioRef.current.load();
			audioRef.current = null;
		}
	}, []);

	const clearBound = useCallback(() => {
		boundRef.current = null;
		setBoundActive(false);
	}, []);

	const stop = useCallback(() => {
		clearBound();
		generationRef.current += 1;
		stopSource();
		playingRef.current = false;
		setPlaying(false);
	}, [clearBound, stopSource]);

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

	const startAt = useCallback(
		(requestedPosition: number, autoplay: boolean) => {
			generationRef.current += 1;
			const generation = generationRef.current;
			stopSource();
			const durationMs = durationRef.current;
			const position = clampPlaybackPosition(requestedPosition, durationMs);
			positionRef.current = position;
			setPositionMs(position);
			setSeekPreviewMs(null);
			if (!autoplay || position >= durationMs) {
				playingRef.current = false;
				setPlaying(false);
				return;
			}
			const segment = segmentAtPosition(segmentsRef.current, position);
			if (!segment) {
				activeSegmentRef.current = null;
				playingRef.current = false;
				setPlaying(false);
				clearBound();
				return;
			}
			activeSegmentRef.current = segment;
			playingRef.current = true;
			setPlaying(true);
			const segmentLimit = Math.min(segment.end_ms, durationMs);
			if (segment.kind === "silence") {
				const wallStart = performance.now();
				const logicalStart = position;
				const tick = (wallNow: number) => {
					if (generationRef.current !== generation || !playingRef.current)
						return;
					const next = logicalStart + (wallNow - wallStart) * rateRef.current;
					if (applyBound(next)) return;
					if (next >= segmentLimit) {
						startAtRef.current(segmentLimit, segmentLimit < durationMs);
						return;
					}
					positionRef.current = next;
					setPositionMs(next);
					animationRef.current = requestAnimationFrame(tick);
				};
				animationRef.current = requestAnimationFrame(tick);
				return;
			}
			const mediaUrl = segment.media_url;
			if (!mediaUrl) {
				if (!applyBound(segmentLimit)) {
					startAtRef.current(segmentLimit, segmentLimit < durationMs);
				}
				return;
			}
			const audio = new Audio();
			audio.crossOrigin = "use-credentials";
			audio.preload = "auto";
			audio.volume = volumeRef.current;
			audio.playbackRate = rateRef.current;
			audioRef.current = audio;
			const failSegment = (message: string) => {
				onErrorRef.current(message);
				playingRef.current = false;
				setPlaying(false);
				clearBound();
			};
			const begin = () => {
				if (generationRef.current !== generation) return;
				mediaRetryRef.current = false;
				const localSeconds = Math.max(0, (position - segment.start_ms) / 1_000);
				audio.currentTime = Number.isFinite(audio.duration)
					? Math.min(localSeconds, Math.max(0, audio.duration - 0.01))
					: localSeconds;
				void audio.play().catch((error: unknown) => {
					if (generationRef.current !== generation) return;
					if (error instanceof DOMException && error.name === "AbortError")
						return;
					failSegment("Browser blocked or failed audio playback.");
				});
			};
			const retryOrFail = () => {
				const generationMatches = generationRef.current === generation;
				if (!shouldRetryMediaLoad(mediaRetryRef.current, generationMatches)) {
					if (generationMatches) {
						failSegment(
							`Could not load segment ${segment.segment_index ?? ""}.`,
						);
					}
					return;
				}
				mediaRetryRef.current = true;
				void refreshForMediaRetry().then((ok) => {
					if (generationRef.current !== generation) return;
					if (ok) {
						onErrorRef.current(null);
						startAtRef.current(positionRef.current, true);
					} else {
						failSegment(SESSION_EXPIRED_MESSAGE);
					}
				});
			};
			audio.addEventListener("pause", () => {
				if (generationRef.current !== generation) return;
				playingRef.current = false;
				setPlaying(false);
			});
			audio.addEventListener("timeupdate", () => {
				if (generationRef.current !== generation) return;
				const logical = segment.start_ms + audio.currentTime * 1_000;
				if (applyBound(logical)) return;
				if (logical >= segmentLimit - 20) {
					if (!applyBound(segmentLimit)) {
						startAtRef.current(segmentLimit, segmentLimit < durationMs);
					}
					return;
				}
				positionRef.current = logical;
				setPositionMs(logical);
			});
			audio.addEventListener("ended", () => {
				if (generationRef.current !== generation) return;
				if (!applyBound(segmentLimit)) {
					startAtRef.current(segmentLimit, segmentLimit < durationMs);
				}
			});
			audio.addEventListener("error", retryOrFail);
			if (segment.kind === "active_hls" && segment.hls_playlist_url) {
				const hlsUrl = absoluteMediaUrl(segment.hls_playlist_url);
				if (audio.canPlayType("application/vnd.apple.mpegurl") === "probably") {
					audio.src = hlsUrl;
					audio.addEventListener("loadedmetadata", begin, { once: true });
				} else {
					void import("hls.js").then(({ default: HlsClass }) => {
						if (generationRef.current !== generation) return;
						if (!HlsClass.isSupported()) {
							audio.src = absoluteMediaUrl(mediaUrl);
							audio.addEventListener("loadedmetadata", begin, { once: true });
							return;
						}
						const hls = new HlsClass({
							xhrSetup: (request) => {
								request.withCredentials = true;
							},
							liveSyncDuration: 2,
							liveMaxLatencyDuration: Number.MAX_SAFE_INTEGER,
						});
						hlsRef.current = hls;
						hls.on(HlsClass.Events.MANIFEST_PARSED, begin);
						hls.on(HlsClass.Events.ERROR, (_event, data) => {
							if (data.fatal) retryOrFail();
						});
						hls.loadSource(hlsUrl);
						hls.attachMedia(audio);
					});
				}
			} else {
				audio.src = absoluteMediaUrl(mediaUrl);
				audio.addEventListener("loadedmetadata", begin, { once: true });
			}
		},
		[applyBound, clearBound, stopSource],
	);
	startAtRef.current = startAt;

	const seekWithinSource = useCallback((position: number) => {
		const targetSegment = segmentAtPosition(segmentsRef.current, position);
		const audio = audioRef.current;
		if (
			playingRef.current &&
			audio &&
			isSameMediaSegment(activeSegmentRef.current, targetSegment)
		) {
			try {
				const localSeconds = Math.max(
					0,
					(position - (targetSegment?.start_ms ?? 0)) / 1_000,
				);
				audio.currentTime = Number.isFinite(audio.duration)
					? Math.min(localSeconds, Math.max(0, audio.duration - 0.01))
					: localSeconds;
				positionRef.current = position;
				setPositionMs(position);
				return;
			} catch {
				// Replacing source below handles media that cannot seek in place.
			}
		}
		startAtRef.current(position, playingRef.current);
	}, []);

	const seek = useCallback(
		(nextPositionMs: number, selection: SessionSelection, loop: boolean) => {
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
			seekWithinSource(target);
		},
		[clearBound, seekWithinSource],
	);

	const togglePlay = useCallback(
		(selection: SessionSelection, loop: boolean) => {
			if (playingRef.current) {
				stop();
				return;
			}
			onErrorRef.current(null);
			if (loop && selection[1] > selection[0]) {
				boundRef.current = { stopMs: selection[1], loopToMs: selection[0] };
				setBoundActive(true);
				const current = positionRef.current;
				startAtRef.current(
					selectionContainsPosition(selection, current)
						? current
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

	const restartWithSegments = useCallback((segments: PlaybackSegment[]) => {
		segmentsRef.current = segments;
		startAtRef.current(positionRef.current, playingRef.current);
	}, []);

	useEffect(
		() => () => {
			generationRef.current += 1;
			stopSource();
		},
		[stopSource],
	);

	return {
		positionMs,
		seekPreviewMs,
		setSeekPreviewMs,
		playing,
		boundActive,
		startAt,
		seek,
		togglePlay,
		togglePreview,
		updateLoop,
		syncBound,
		clearBound,
		stop,
		restartWithSegments,
	};
}

export type SegmentedSessionPlayback = ReturnType<
	typeof useSegmentedSessionPlayback
>;
