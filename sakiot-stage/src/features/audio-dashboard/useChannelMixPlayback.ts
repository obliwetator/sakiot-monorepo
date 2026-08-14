import type Hls from "hls.js";
import { useCallback, useEffect, useRef, useState } from "react";
import type {
	ChannelMixParticipantSettings,
	ChannelMixSourceSegment,
	ChannelMixTrack,
} from "../../app/apiSlice";
import { BASE_API_URL } from "../../app/apiSlice";
import {
	refreshForMediaRetry,
	SESSION_EXPIRED_MESSAGE,
} from "../../app/authedFetch";
import { commonLiveSeekPosition } from "./channelMixState";

const CHANNEL_MIX_SAMPLE_RATE = 48_000;
const SCHEDULE_AHEAD_MS = 8_000;
const DRIFT_LIMIT_MS = 150;

interface SourceState {
	segment: ChannelMixSourceSegment;
	trackUserId: string;
	audio: HTMLAudioElement | null;
	mediaSource: MediaElementAudioSourceNode | null;
	participantGain: GainNode | null;
	monitorGain: GainNode | null;
	hls: Hls | null;
	attachedLive: boolean;
	lastDriftCorrectionAt: number;
	retried: boolean;
	failed: boolean;
	shouldPlay: boolean;
}

interface Transport {
	contextStartTime: number;
	positionMs: number;
	rate: number;
}

interface ChannelMixPlaybackOptions {
	tracks: readonly ChannelMixTrack[];
	durationMs: number;
	settings: readonly ChannelMixParticipantSettings[];
	volume: number;
	playbackRate: number;
	onSourceError?: (segmentId: string, message: string | null) => void;
}

function absoluteMediaUrl(path: string): string {
	return new URL(
		path,
		new URL(BASE_API_URL, window.location.origin),
	).toString();
}

function gainForParticipant(
	userId: string,
	settings: readonly ChannelMixParticipantSettings[],
): number {
	const setting = settings.find((item) => item.user_id === userId);
	if (!setting || setting.muted) return 0;
	return 10 ** (setting.gain_db / 20);
}

function setAudioParam(
	param: AudioParam,
	value: number,
	context: AudioContext,
): void {
	param.cancelScheduledValues(context.currentTime);
	param.setTargetAtTime(value, context.currentTime, 0.01);
}

export function useChannelMixPlayback(options: ChannelMixPlaybackOptions) {
	const [positionMs, setPositionMs] = useState(0);
	const [playing, setPlaying] = useState(false);
	const [sourceErrors, setSourceErrors] = useState<Record<string, string>>({});
	const [followingLive, setFollowingLive] = useState(false);
	const tracksRef = useRef(options.tracks);
	const settingsRef = useRef(options.settings);
	const durationRef = useRef(options.durationMs);
	const volumeRef = useRef(options.volume);
	const rateRef = useRef(options.playbackRate);
	const positionRef = useRef(0);
	const playingRef = useRef(false);
	const followingLiveRef = useRef(false);
	const animationRef = useRef<number | null>(null);
	const commandRef = useRef(0);
	const sourcesRef = useRef(new Map<string, SourceState>());
	const participantGainsRef = useRef(new Map<string, GainNode>());
	const contextRef = useRef<AudioContext | null>(null);
	const masterGainRef = useRef<GainNode | null>(null);
	const transportRef = useRef<Transport | null>(null);

	const reportError = useCallback(
		(segmentId: string, message: string | null) => {
			setSourceErrors((current) => {
				if (message === null) {
					if (!(segmentId in current)) return current;
					const next = { ...current };
					delete next[segmentId];
					return next;
				}
				return { ...current, [segmentId]: message };
			});
			options.onSourceError?.(segmentId, message);
		},
		[options.onSourceError],
	);

	const ensureAudioGraph = useCallback(() => {
		if (contextRef.current) return contextRef.current;
		const context = new AudioContext({ sampleRate: CHANNEL_MIX_SAMPLE_RATE });
		const masterGain = context.createGain();
		masterGain.gain.value = volumeRef.current;
		const limiter = context.createDynamicsCompressor();
		limiter.threshold.value = -1;
		limiter.knee.value = 0;
		limiter.ratio.value = 20;
		limiter.attack.value = 0.003;
		limiter.release.value = 0.05;
		masterGain.connect(limiter).connect(context.destination);
		contextRef.current = context;
		masterGainRef.current = masterGain;
		return context;
	}, []);

	const sourceFor = useCallback(
		(segment: ChannelMixSourceSegment, trackUserId: string): SourceState => {
			const existing = sourcesRef.current.get(segment.id);
			if (existing) {
				existing.segment = segment;
				existing.trackUserId = trackUserId;
				return existing;
			}
			const source: SourceState = {
				segment,
				trackUserId,
				audio: null,
				mediaSource: null,
				participantGain: null,
				monitorGain: null,
				hls: null,
				attachedLive: false,
				lastDriftCorrectionAt: Number.NEGATIVE_INFINITY,
				retried: false,
				failed: false,
				shouldPlay: false,
			};
			sourcesRef.current.set(segment.id, source);
			return source;
		},
		[],
	);

	const ensureParticipantGain = useCallback(
		(userId: string, context: AudioContext): GainNode => {
			const existing = participantGainsRef.current.get(userId);
			if (existing) return existing;
			const gain = context.createGain();
			gain.gain.value = gainForParticipant(userId, settingsRef.current);
			const masterGain = masterGainRef.current;
			if (!masterGain)
				throw new Error("Channel mix master graph is unavailable");
			gain.connect(masterGain);
			participantGainsRef.current.set(userId, gain);
			return gain;
		},
		[],
	);

	const ensureMonitorGain = useCallback(
		(source: SourceState, context: AudioContext): GainNode => {
			if (source.monitorGain) return source.monitorGain;
			const gain = context.createGain();
			gain.gain.value = 1;
			const participantGain = ensureParticipantGain(
				source.trackUserId,
				context,
			);
			gain.connect(participantGain);
			source.monitorGain = gain;
			return gain;
		},
		[ensureParticipantGain],
	);

	const updateGains = useCallback(() => {
		const context = contextRef.current;
		if (!context) return;
		for (const [userId, gain] of participantGainsRef.current) {
			setAudioParam(
				gain.gain,
				gainForParticipant(userId, settingsRef.current),
				context,
			);
		}
	}, []);

	const readPosition = useCallback((): number => {
		const context = contextRef.current;
		const transport = transportRef.current;
		if (!context || !transport) return positionRef.current;
		return (
			transport.positionMs +
			Math.max(0, context.currentTime - transport.contextStartTime) *
				1_000 *
				transport.rate
		);
	}, []);

	const setTransport = useCallback(
		(position: number, context: AudioContext) => {
			positionRef.current = position;
			transportRef.current = {
				contextStartTime: context.currentTime,
				positionMs: position,
				rate: rateRef.current,
			};
		},
		[],
	);

	const pauseLiveSources = useCallback(() => {
		for (const source of sourcesRef.current.values()) {
			if (!source.audio) continue;
			source.shouldPlay = false;
			source.audio.pause();
		}
	}, []);

	const liveDesiredSeconds = useCallback(
		(source: SourceState, position: number): number =>
			Math.max(
				0,
				(position - source.segment.start_ms + source.segment.source_offset_ms) /
					1_000,
			),
		[],
	);

	const startLiveSource = useCallback(
		(source: SourceState, position: number) => {
			if (!source.audio || !source.shouldPlay || source.failed) return;
			const desiredSeconds = liveDesiredSeconds(source, position);
			try {
				if (
					Number.isFinite(source.audio.duration) &&
					source.audio.duration > 0
				) {
					source.audio.currentTime = Math.min(
						desiredSeconds,
						Math.max(0, source.audio.duration - 0.01),
					);
				} else {
					source.audio.currentTime = desiredSeconds;
				}
				source.audio.playbackRate = rateRef.current;
				void source.audio.play().catch(() => {
					reportError(
						source.segment.id,
						"Browser blocked this source playback.",
					);
				});
			} catch {
				reportError(source.segment.id, "This source could not be seeked.");
			}
		},
		[liveDesiredSeconds, reportError],
	);

	const attachLiveSource = useCallback(
		(source: SourceState, context: AudioContext) => {
			if (source.attachedLive) return;
			source.attachedLive = true;
			source.audio = new Audio();
			source.audio.crossOrigin = "use-credentials";
			source.audio.preload = "auto";
			source.audio.playbackRate = rateRef.current;
			try {
				source.mediaSource = context.createMediaElementSource(source.audio);
			} catch {
				// The element still has a direct fallback if Web Audio rejects it.
			}
			try {
				const mediaSource = source.mediaSource;
				if (mediaSource)
					mediaSource.connect(ensureMonitorGain(source, context));
			} catch {
				// Keep loading the source even if a browser rejects the graph node.
			}
			const retrySource = () => {
				if (!source.audio || source.failed) return;
				if (source.retried) {
					source.failed = true;
					reportError(source.segment.id, "This source failed while loading.");
					return;
				}
				source.retried = true;
				void refreshForMediaRetry().then((ok) => {
					if (!source.audio) return;
					if (!ok) {
						source.failed = true;
						reportError(source.segment.id, SESSION_EXPIRED_MESSAGE);
						return;
					}
					reportError(source.segment.id, null);
					if (source.hls) source.hls.startLoad();
					source.audio.load();
				});
			};
			source.audio.addEventListener("canplay", () => {
				reportError(source.segment.id, null);
				if (source.shouldPlay) startLiveSource(source, readPosition());
			});
			source.audio.addEventListener("error", retrySource);
			if (source.segment.live && source.segment.hls_playlist_url) {
				const hlsUrl = absoluteMediaUrl(source.segment.hls_playlist_url);
				if (
					source.audio.canPlayType("application/vnd.apple.mpegurl") ===
					"probably"
				) {
					source.audio.src = hlsUrl;
				} else {
					void import("hls.js").then(({ default: HlsClass }) => {
						if (!source.audio) return;
						if (!HlsClass.isSupported()) {
							source.audio.src = absoluteMediaUrl(source.segment.media_url);
							return;
						}
						const hls = new HlsClass({
							xhrSetup: (request) => {
								request.withCredentials = true;
							},
							liveSyncDuration: 2,
						});
						source.hls = hls;
						hls.on(HlsClass.Events.ERROR, (_event, data) => {
							if (data.fatal) retrySource();
						});
						hls.loadSource(hlsUrl);
						hls.attachMedia(source.audio);
					});
				}
			} else {
				source.audio.src = absoluteMediaUrl(source.segment.media_url);
			}
		},
		[ensureMonitorGain, readPosition, reportError, startLiveSource],
	);

	const updateLiveSources = useCallback(
		(position: number, context: AudioContext) => {
			for (const source of sourcesRef.current.values()) {
				if (source.audio) source.shouldPlay = false;
			}
			const activeSegmentIds = new Set<string>();
			for (const track of tracksRef.current) {
				let selected: ChannelMixSourceSegment | null = null;
				for (const segment of track.segments) {
					if (
						position < segment.start_ms ||
						position >= segment.end_ms ||
						sourceFor(segment, track.user_id).failed
					)
						continue;
					if (
						selected === null ||
						segment.start_ms > selected.start_ms ||
						(segment.start_ms === selected.start_ms &&
							segment.end_ms > selected.end_ms)
					) {
						selected = segment;
					}
				}
				if (selected) activeSegmentIds.add(selected.id);
			}
			const active = new Set<string>();
			for (const track of tracksRef.current) {
				for (const segment of track.segments) {
					const source = sourceFor(segment, track.user_id);
					if (
						source.failed ||
						segment.end_ms <= position ||
						segment.start_ms > position + SCHEDULE_AHEAD_MS
					)
						continue;
					attachLiveSource(source, context);
					const activeNow = activeSegmentIds.has(segment.id);
					source.shouldPlay = activeNow;
					if (!activeNow || !source.audio) continue;
					active.add(segment.id);
					if (source.audio.readyState >= HTMLMediaElement.HAVE_FUTURE_DATA) {
						const desiredSeconds = liveDesiredSeconds(source, position);
						if (
							Math.abs(source.audio.currentTime - desiredSeconds) * 1_000 >
								DRIFT_LIMIT_MS &&
							context.currentTime - source.lastDriftCorrectionAt > 0.5
						) {
							try {
								source.audio.currentTime = desiredSeconds;
								source.lastDriftCorrectionAt = context.currentTime;
							} catch {
								// HLS can briefly make its seekable range unavailable.
							}
						}
						if (source.audio.paused) startLiveSource(source, position);
					}
				}
			}
			for (const source of sourcesRef.current.values()) {
				if (!source.audio || active.has(source.segment.id)) continue;
				source.audio.pause();
			}
		},
		[attachLiveSource, liveDesiredSeconds, sourceFor, startLiveSource],
	);

	const finish = useCallback(() => {
		commandRef.current += 1;
		playingRef.current = false;
		followingLiveRef.current = false;
		setFollowingLive(false);
		setPlaying(false);
		positionRef.current = durationRef.current;
		setPositionMs(durationRef.current);
		transportRef.current = null;
		pauseLiveSources();
	}, [pauseLiveSources]);

	const tick = useCallback(() => {
		if (!playingRef.current) return;
		const position = readPosition();
		if (position >= durationRef.current) {
			finish();
			return;
		}
		positionRef.current = position;
		setPositionMs(position);
		const context = contextRef.current;
		if (context) updateLiveSources(position, context);
		animationRef.current = requestAnimationFrame(() => tick());
	}, [finish, readPosition, updateLiveSources]);

	const restartAt = useCallback(
		(position: number) => {
			const context = contextRef.current;
			if (!context || !playingRef.current) return;
			pauseLiveSources();
			setTransport(position, context);
			updateLiveSources(position, context);
			if (animationRef.current !== null)
				cancelAnimationFrame(animationRef.current);
			animationRef.current = requestAnimationFrame(() => tick());
		},
		[pauseLiveSources, setTransport, tick, updateLiveSources],
	);

	const pause = useCallback(() => {
		const position = Math.min(durationRef.current, readPosition());
		commandRef.current += 1;
		playingRef.current = false;
		setPlaying(false);
		positionRef.current = position;
		setPositionMs(position);
		transportRef.current = null;
		pauseLiveSources();
	}, [pauseLiveSources, readPosition]);

	const start = useCallback(async () => {
		const command = ++commandRef.current;
		const context = ensureAudioGraph();
		try {
			if (context.state === "suspended") await context.resume();
		} catch {
			setSourceErrors((current) => ({
				...current,
				master: "The browser could not start the audio mixer.",
			}));
			return;
		}
		if (commandRef.current !== command) return;
		let position = positionRef.current;
		if (position >= durationRef.current) position = 0;
		pauseLiveSources();
		setTransport(position, context);
		playingRef.current = true;
		setPlaying(true);
		updateLiveSources(position, context);
		if (animationRef.current !== null)
			cancelAnimationFrame(animationRef.current);
		animationRef.current = requestAnimationFrame(() => tick());
	}, [
		ensureAudioGraph,
		pauseLiveSources,
		setTransport,
		tick,
		updateLiveSources,
	]);

	const seek = useCallback(
		(nextPositionMs: number) => {
			const next = Math.max(0, Math.min(durationRef.current, nextPositionMs));
			followingLiveRef.current = false;
			setFollowingLive(false);
			positionRef.current = next;
			setPositionMs(next);
			if (playingRef.current) restartAt(next);
		},
		[restartAt],
	);

	const togglePlay = useCallback(() => {
		if (playingRef.current) {
			pause();
		} else {
			void start();
		}
	}, [pause, start]);

	const goLive = useCallback(() => {
		const edges: number[] = [];
		for (const track of tracksRef.current) {
			const muted = settingsRef.current.find(
				(setting) => setting.user_id === track.user_id,
			)?.muted;
			const audibleSegments = muted ? [] : track.segments;
			if (audibleSegments.length === 0) continue;
			const segment = audibleSegments.reduce((latest, candidate) =>
				candidate.end_ms > latest.end_ms ? candidate : latest,
			);
			const source = sourceFor(segment, track.user_id);
			let edge = segment.end_ms;
			if (segment.live && source.audio && source.audio.seekable.length > 0) {
				try {
					const sourceEdgeMs =
						source.audio.seekable.end(source.audio.seekable.length - 1) * 1_000;
					edge = segment.start_ms + sourceEdgeMs - segment.source_offset_ms;
				} catch {
					// The HLS seekable window can disappear during a playlist refresh.
				}
			}
			if (Number.isFinite(edge)) edges.push(edge);
		}
		const target = commonLiveSeekPosition(edges);
		if (target === null) return;
		seek(target);
		followingLiveRef.current = true;
		setFollowingLive(true);
		if (!playingRef.current) void start();
	}, [seek, sourceFor, start]);

	useEffect(() => {
		tracksRef.current = options.tracks;
	}, [options.tracks]);

	useEffect(() => {
		durationRef.current = options.durationMs;
		if (positionRef.current > options.durationMs) {
			positionRef.current = options.durationMs;
			setPositionMs(options.durationMs);
		}
	}, [options.durationMs]);

	useEffect(() => {
		settingsRef.current = options.settings;
		updateGains();
	}, [options.settings, updateGains]);

	useEffect(() => {
		volumeRef.current = options.volume;
		const context = contextRef.current;
		const masterGain = masterGainRef.current;
		if (context && masterGain)
			setAudioParam(masterGain.gain, options.volume, context);
	}, [options.volume]);

	useEffect(() => {
		if (rateRef.current === options.playbackRate) return;
		rateRef.current = options.playbackRate;
		for (const source of sourcesRef.current.values()) {
			if (source.audio) source.audio.playbackRate = options.playbackRate;
		}
		if (playingRef.current) restartAt(readPosition());
	}, [options.playbackRate, readPosition, restartAt]);

	useEffect(() => {
		const known = new Set(
			options.tracks.flatMap((track) =>
				track.segments.map((segment) => segment.id),
			),
		);
		for (const [id, source] of sourcesRef.current) {
			if (known.has(id)) continue;
			source.hls?.destroy();
			source.audio?.pause();
			source.audio?.removeAttribute("src");
			source.audio?.load();
			sourcesRef.current.delete(id);
		}
		updateGains();
	}, [options.tracks, updateGains]);

	useEffect(() => {
		if (
			options.tracks.length === 0 ||
			!followingLiveRef.current ||
			!playingRef.current
		)
			return;
		const timeout = window.setTimeout(() => goLive(), 0);
		return () => window.clearTimeout(timeout);
	}, [goLive, options.tracks]);

	useEffect(
		() => () => {
			commandRef.current += 1;
			if (animationRef.current !== null)
				cancelAnimationFrame(animationRef.current);
			for (const source of sourcesRef.current.values()) {
				source.hls?.destroy();
				source.audio?.pause();
				source.audio?.removeAttribute("src");
				source.audio?.load();
			}
			void contextRef.current?.close();
		},
		[],
	);

	return {
		positionMs,
		playing,
		followingLive,
		sourceErrors,
		seek,
		togglePlay,
		goLive,
		stop: pause,
	};
}
