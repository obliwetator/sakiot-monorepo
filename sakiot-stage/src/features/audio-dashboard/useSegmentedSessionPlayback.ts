import type Hls from "hls.js";
import { useCallback, useEffect, useRef, useState } from "react";
import { absoluteMediaUrl } from "../../api/routes";
import {
	refreshForMediaRetry,
	SESSION_EXPIRED_MESSAGE,
} from "../../app/authedFetch";
import { attachHlsAudio, prefersNativeHls } from "../../shared/attachHls";
import {
	clampPlaybackPosition,
	isSameMediaSegment,
	segmentAtPosition,
	shouldRetryMediaLoad,
} from "./logicalSessionPlaybackState";
import type { SessionSelection } from "./logicalSessionSelection";
import type { PlaybackSegment } from "./logicalSessionTimeline";
import { usePlaybackBound } from "./usePlaybackBound";

interface SegmentedPlaybackOptions {
	segments: PlaybackSegment[];
	durationMs: number;
	volume: number;
	playbackRate: number;
	onError: (message: string | null) => void;
	onLoopDisabled: () => void;
}

export function useSegmentedSessionPlayback(options: SegmentedPlaybackOptions) {
	const [positionMs, setPositionMs] = useState(0);
	const [seekPreviewMs, setSeekPreviewMs] = useState<number | null>(null);
	const [playing, setPlaying] = useState(false);
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
	const startAtRef = useRef<(position: number, autoplay: boolean) => void>(
		() => {},
	);
	const onErrorRef = useRef(options.onError);
	const onLoopDisabledRef = useRef(options.onLoopDisabled);
	const stopRef = useRef<() => void>(() => {});
	const onBeforePlayRef = useRef<() => void>(() => {});
	onBeforePlayRef.current = () => onErrorRef.current(null);

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

	const stop = useCallback(() => {
		bound.clearBound();
		generationRef.current += 1;
		stopSource();
		playingRef.current = false;
		setPlaying(false);
	}, [bound.clearBound, stopSource]);
	stopRef.current = stop;

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
				bound.clearBound();
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
					if (bound.applyBound(next)) return;
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
				if (!bound.applyBound(segmentLimit)) {
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
				bound.clearBound();
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
			// `timeupdate` only fires a few times a second, so a playhead driven
			// by it alone steps over audio while the silence branches, advanced
			// per animation frame, glide. Read the media clock every frame and
			// keep `timeupdate` as the fallback for throttled or hidden tabs.
			const syncFromAudio = () => {
				if (generationRef.current !== generation) return false;
				const mediaSeconds = audio.currentTime;
				if (!Number.isFinite(mediaSeconds)) return true;
				const logical = segment.start_ms + mediaSeconds * 1_000;
				if (bound.applyBound(logical)) return false;
				if (logical >= segmentLimit - 20) {
					if (!bound.applyBound(segmentLimit)) {
						startAtRef.current(segmentLimit, segmentLimit < durationMs);
					}
					return false;
				}
				positionRef.current = logical;
				setPositionMs(logical);
				return true;
			};
			const tickFromAudio = () => {
				animationRef.current = null;
				if (generationRef.current !== generation || !playingRef.current) return;
				if (!syncFromAudio()) return;
				animationRef.current = requestAnimationFrame(tickFromAudio);
			};
			const startAudioTicker = () => {
				if (generationRef.current !== generation) return;
				if (animationRef.current !== null) return;
				animationRef.current = requestAnimationFrame(tickFromAudio);
			};
			audio.addEventListener("playing", startAudioTicker);
			audio.addEventListener("timeupdate", () => {
				if (generationRef.current !== generation) return;
				syncFromAudio();
			});
			audio.addEventListener("ended", () => {
				if (generationRef.current !== generation) return;
				if (!bound.applyBound(segmentLimit)) {
					startAtRef.current(segmentLimit, segmentLimit < durationMs);
				}
			});
			audio.addEventListener("error", retryOrFail);
			if (segment.kind === "active_hls" && segment.hls_playlist_url) {
				const hlsUrl = absoluteMediaUrl(segment.hls_playlist_url);
				if (prefersNativeHls(audio)) {
					audio.src = hlsUrl;
					audio.addEventListener("loadedmetadata", begin, { once: true });
				} else {
					void attachHlsAudio({
						audio,
						playlistUrl: hlsUrl,
						fallbackUrl: absoluteMediaUrl(mediaUrl),
						isActive: () => generationRef.current === generation,
						onFatal: () => retryOrFail(),
						onManifestParsed: begin,
						unlimitedMaxLatency: true,
					}).then((result) => {
						if (generationRef.current !== generation) return;
						if (result.kind === "direct") {
							audio.addEventListener("loadedmetadata", begin, { once: true });
						}
						hlsRef.current = result.hls;
					});
				}
			} else {
				audio.src = absoluteMediaUrl(mediaUrl);
				audio.addEventListener("loadedmetadata", begin, { once: true });
			}
		},
		[bound.applyBound, bound.clearBound, stopSource],
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
			bound.prepareSeek(selection, loop, target);
			seekWithinSource(target);
		},
		[bound.prepareSeek, seekWithinSource],
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
		boundActive: bound.boundActive,
		startAt,
		seek,
		togglePlay: bound.togglePlay,
		togglePreview: bound.togglePreview,
		updateLoop: bound.updateLoop,
		syncBound: bound.syncBound,
		clearBound: bound.clearBound,
		stop,
		restartWithSegments,
	};
}

export type SegmentedSessionPlayback = ReturnType<
	typeof useSegmentedSessionPlayback
>;
