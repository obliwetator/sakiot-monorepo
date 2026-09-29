import type Hls from "hls.js";
import { absoluteMediaUrl } from "../../api/routes";
import type {
	ChannelMixParticipantSettings,
	ChannelMixSourceSegment,
	ChannelMixTrack,
} from "../../app/apiSlice";
import {
	refreshForMediaRetry,
	SESSION_EXPIRED_MESSAGE,
} from "../../app/authedFetch";
import { attachHlsAudio, prefersNativeHls } from "../../shared/attachHls";
import { commonLiveSeekPosition, shouldSeekSource } from "./channelMixState";
import { PlaybackStore } from "./playbackStore";

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

export interface ChannelMixOptions {
	tracks: readonly ChannelMixTrack[];
	durationMs: number;
	settings: readonly ChannelMixParticipantSettings[];
	volume: number;
	playbackRate: number;
	onSourceError?: (segmentId: string, message: string | null) => void;
}

export interface ChannelMixSnapshot {
	readonly positionMs: number;
	readonly playing: boolean;
	readonly followingLive: boolean;
	readonly sourceErrors: Readonly<Record<string, string>>;
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

function releaseSource(source: SourceState): void {
	source.hls?.destroy();
	source.audio?.pause();
	source.audio?.removeAttribute("src");
	source.audio?.load();
}

/**
 * Plays an unrendered channel mix: one media element per source segment,
 * routed through a per-participant gain into a limited master bus and kept on
 * one transport clock, so the preview can follow a mix that is still live.
 *
 * The engine owns every piece of mutable playback state; React reads it
 * through the snapshot. `dispose` releases the audio graph but leaves the
 * engine usable, which Strict Mode's mount, unmount, mount sequence needs.
 */
export class ChannelMixEngine extends PlaybackStore<ChannelMixSnapshot> {
	private tracks: readonly ChannelMixTrack[];
	private settings: readonly ChannelMixParticipantSettings[];
	private durationMs: number;
	private volume: number;
	private rate: number;
	private onSourceError: ChannelMixOptions["onSourceError"];
	private animation: number | null = null;
	private command = 0;
	private liveFollowTimer: number | null = null;
	private readonly sources = new Map<string, SourceState>();
	private readonly participantGains = new Map<string, GainNode>();
	private context: AudioContext | null = null;
	private masterGain: GainNode | null = null;
	private transport: Transport | null = null;

	constructor(options: ChannelMixOptions) {
		super({
			positionMs: 0,
			playing: false,
			followingLive: false,
			sourceErrors: {},
		});
		this.tracks = options.tracks;
		this.settings = options.settings;
		this.durationMs = options.durationMs;
		this.volume = options.volume;
		this.rate = options.playbackRate;
		this.onSourceError = options.onSourceError;
	}

	setTracks(tracks: readonly ChannelMixTrack[]): void {
		if (tracks === this.tracks) return;
		this.tracks = tracks;
		const known = new Set(
			tracks.flatMap((track) => track.segments.map((segment) => segment.id)),
		);
		for (const [id, source] of this.sources) {
			if (known.has(id)) continue;
			releaseSource(source);
			this.sources.delete(id);
		}
		this.updateGains();
		this.clearLiveFollowTimer();
		// A live mix grows; following it means re-seeking to the new edge.
		if (
			tracks.length > 0 &&
			this.snapshot.followingLive &&
			this.snapshot.playing
		) {
			this.liveFollowTimer = window.setTimeout(() => {
				this.liveFollowTimer = null;
				this.goLive();
			}, 0);
		}
	}

	setDurationMs(durationMs: number): void {
		this.durationMs = durationMs;
		if (this.snapshot.positionMs > durationMs) {
			this.publish({ positionMs: durationMs });
		}
	}

	setSettings(settings: readonly ChannelMixParticipantSettings[]): void {
		this.settings = settings;
		this.updateGains();
	}

	setVolume(volume: number): void {
		this.volume = volume;
		if (this.context && this.masterGain) {
			setAudioParam(this.masterGain.gain, volume, this.context);
		}
	}

	setPlaybackRate(rate: number): void {
		if (rate === this.rate) return;
		this.rate = rate;
		for (const source of this.sources.values()) {
			if (source.audio) source.audio.playbackRate = rate;
		}
		if (this.snapshot.playing) this.restartAt(this.readPosition());
	}

	setSourceErrorListener(listener: ChannelMixOptions["onSourceError"]): void {
		this.onSourceError = listener;
	}

	readonly seek = (nextPositionMs: number): void => {
		const next = Math.max(0, Math.min(this.durationMs, nextPositionMs));
		this.publish({ followingLive: false, positionMs: next });
		if (this.snapshot.playing) this.restartAt(next);
	};

	readonly togglePlay = (): void => {
		if (this.snapshot.playing) {
			this.pause();
		} else {
			void this.start();
		}
	};

	readonly pause = (): void => {
		const position = Math.min(this.durationMs, this.readPosition());
		this.command += 1;
		this.publish({ playing: false, positionMs: position });
		this.transport = null;
		this.pauseLiveSources();
	};

	readonly goLive = (): void => {
		const edges: number[] = [];
		for (const track of this.tracks) {
			const muted = this.settings.find(
				(setting) => setting.user_id === track.user_id,
			)?.muted;
			const audibleSegments = muted ? [] : track.segments;
			if (audibleSegments.length === 0) continue;
			const segment = audibleSegments.reduce((latest, candidate) =>
				candidate.end_ms > latest.end_ms ? candidate : latest,
			);
			const source = this.sourceFor(segment, track.user_id);
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
		this.seek(target);
		this.publish({ followingLive: true });
		if (!this.snapshot.playing) void this.start();
	};

	/** Releases the audio graph and every source; the engine stays reusable. */
	dispose(): void {
		this.command += 1;
		if (this.animation !== null) cancelAnimationFrame(this.animation);
		this.animation = null;
		this.clearLiveFollowTimer();
		for (const source of this.sources.values()) releaseSource(source);
		this.sources.clear();
		this.participantGains.clear();
		void this.context?.close();
		this.context = null;
		this.masterGain = null;
		this.transport = null;
		if (this.snapshot.playing) this.publish({ playing: false });
	}

	private async start(): Promise<void> {
		const command = ++this.command;
		const context = this.ensureAudioGraph();
		try {
			if (context.state === "suspended") await context.resume();
		} catch {
			this.publish({
				sourceErrors: {
					...this.snapshot.sourceErrors,
					master: "The browser could not start the audio mixer.",
				},
			});
			return;
		}
		if (this.command !== command) return;
		let position = this.snapshot.positionMs;
		if (position >= this.durationMs) position = 0;
		this.pauseLiveSources();
		this.setTransport(position, context);
		this.publish({ playing: true });
		this.updateLiveSources(position, context);
		this.scheduleTick();
	}

	private finish(): void {
		this.command += 1;
		this.publish({
			followingLive: false,
			playing: false,
			positionMs: this.durationMs,
		});
		this.transport = null;
		this.pauseLiveSources();
	}

	private readonly tick = (): void => {
		if (!this.snapshot.playing) return;
		const position = this.readPosition();
		if (position >= this.durationMs) {
			this.finish();
			return;
		}
		this.publish({ positionMs: position });
		const context = this.context;
		if (context) this.updateLiveSources(position, context);
		this.animation = requestAnimationFrame(this.tick);
	};

	private scheduleTick(): void {
		if (this.animation !== null) cancelAnimationFrame(this.animation);
		this.animation = requestAnimationFrame(this.tick);
	}

	private restartAt(position: number): void {
		const context = this.context;
		if (!context || !this.snapshot.playing) return;
		this.pauseLiveSources();
		this.setTransport(position, context);
		this.updateLiveSources(position, context);
		this.scheduleTick();
	}

	private clearLiveFollowTimer(): void {
		if (this.liveFollowTimer === null) return;
		window.clearTimeout(this.liveFollowTimer);
		this.liveFollowTimer = null;
	}

	private reportError(segmentId: string, message: string | null): void {
		const current = this.snapshot.sourceErrors;
		if (message === null) {
			if (segmentId in current) {
				const next = { ...current };
				delete next[segmentId];
				this.publish({ sourceErrors: next });
			}
		} else {
			this.publish({ sourceErrors: { ...current, [segmentId]: message } });
		}
		this.onSourceError?.(segmentId, message);
	}

	private ensureAudioGraph(): AudioContext {
		if (this.context) return this.context;
		const context = new AudioContext({ sampleRate: CHANNEL_MIX_SAMPLE_RATE });
		const masterGain = context.createGain();
		masterGain.gain.value = this.volume;
		const limiter = context.createDynamicsCompressor();
		limiter.threshold.value = -1;
		limiter.knee.value = 0;
		limiter.ratio.value = 20;
		limiter.attack.value = 0.003;
		limiter.release.value = 0.05;
		masterGain.connect(limiter).connect(context.destination);
		this.context = context;
		this.masterGain = masterGain;
		return context;
	}

	private sourceFor(
		segment: ChannelMixSourceSegment,
		trackUserId: string,
	): SourceState {
		const existing = this.sources.get(segment.id);
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
		this.sources.set(segment.id, source);
		return source;
	}

	private ensureParticipantGain(
		userId: string,
		context: AudioContext,
	): GainNode {
		const existing = this.participantGains.get(userId);
		if (existing) return existing;
		const gain = context.createGain();
		gain.gain.value = gainForParticipant(userId, this.settings);
		const masterGain = this.masterGain;
		if (!masterGain) throw new Error("Channel mix master graph is unavailable");
		gain.connect(masterGain);
		this.participantGains.set(userId, gain);
		return gain;
	}

	private ensureMonitorGain(
		source: SourceState,
		context: AudioContext,
	): GainNode {
		if (source.monitorGain) return source.monitorGain;
		const gain = context.createGain();
		gain.gain.value = 1;
		gain.connect(this.ensureParticipantGain(source.trackUserId, context));
		source.monitorGain = gain;
		return gain;
	}

	private updateGains(): void {
		const context = this.context;
		if (!context) return;
		for (const [userId, gain] of this.participantGains) {
			setAudioParam(
				gain.gain,
				gainForParticipant(userId, this.settings),
				context,
			);
		}
	}

	private readPosition(): number {
		const context = this.context;
		const transport = this.transport;
		if (!context || !transport) return this.snapshot.positionMs;
		return (
			transport.positionMs +
			Math.max(0, context.currentTime - transport.contextStartTime) *
				1_000 *
				transport.rate
		);
	}

	private setTransport(position: number, context: AudioContext): void {
		this.publish({ positionMs: position });
		this.transport = {
			contextStartTime: context.currentTime,
			positionMs: position,
			rate: this.rate,
		};
	}

	private pauseLiveSources(): void {
		for (const source of this.sources.values()) {
			if (!source.audio) continue;
			source.shouldPlay = false;
			source.audio.pause();
		}
	}

	private liveDesiredSeconds(source: SourceState, position: number): number {
		return Math.max(
			0,
			(position - source.segment.start_ms + source.segment.source_offset_ms) /
				1_000,
		);
	}

	private startLiveSource(source: SourceState, position: number): void {
		if (!source.audio || !source.shouldPlay || source.failed) return;
		const desiredSeconds = this.liveDesiredSeconds(source, position);
		const targetSeconds =
			Number.isFinite(source.audio.duration) && source.audio.duration > 0
				? Math.min(desiredSeconds, Math.max(0, source.audio.duration - 0.01))
				: desiredSeconds;
		try {
			// Repositioning a playing element re-fires `canplay`. Only correct a
			// real drift, otherwise a `canplay` re-entry feeds itself.
			if (
				shouldSeekSource(
					source.audio.currentTime,
					targetSeconds,
					source.audio.paused,
					DRIFT_LIMIT_MS,
				)
			) {
				source.audio.currentTime = targetSeconds;
			}
			source.audio.playbackRate = this.rate;
			void source.audio.play().catch(() => {
				this.reportError(
					source.segment.id,
					"Browser blocked this source playback.",
				);
			});
		} catch {
			this.reportError(source.segment.id, "This source could not be seeked.");
		}
	}

	private attachLiveSource(source: SourceState, context: AudioContext): void {
		if (source.attachedLive) return;
		source.attachedLive = true;
		source.audio = new Audio();
		source.audio.crossOrigin = "use-credentials";
		source.audio.preload = "auto";
		source.audio.playbackRate = this.rate;
		try {
			source.mediaSource = context.createMediaElementSource(source.audio);
		} catch {
			// The element still has a direct fallback if Web Audio rejects it.
		}
		try {
			const mediaSource = source.mediaSource;
			if (mediaSource)
				mediaSource.connect(this.ensureMonitorGain(source, context));
		} catch {
			// Keep loading the source even if a browser rejects the graph node.
		}
		const retrySource = () => {
			if (!source.audio || source.failed) return;
			if (source.retried) {
				source.failed = true;
				this.reportError(
					source.segment.id,
					"This source failed while loading.",
				);
				return;
			}
			source.retried = true;
			void refreshForMediaRetry().then((ok) => {
				if (!source.audio) return;
				if (!ok) {
					source.failed = true;
					this.reportError(source.segment.id, SESSION_EXPIRED_MESSAGE);
					return;
				}
				this.reportError(source.segment.id, null);
				if (source.hls) source.hls.startLoad();
				source.audio.load();
			});
		};
		// Starting from `canplay` used to seek the element, which re-fired
		// `canplay` and looped (~500 seeks/second), stuttering every source in
		// the mix. `updateLiveSources` already starts paused, active sources
		// once they are ready, so this listener only clears load errors.
		source.audio.addEventListener("canplay", () => {
			this.reportError(source.segment.id, null);
		});
		source.audio.addEventListener("error", retrySource);
		if (source.segment.live && source.segment.hls_playlist_url) {
			const hlsUrl = absoluteMediaUrl(source.segment.hls_playlist_url);
			if (prefersNativeHls(source.audio)) {
				source.audio.src = hlsUrl;
			} else {
				void attachHlsAudio({
					audio: source.audio,
					playlistUrl: hlsUrl,
					fallbackUrl: absoluteMediaUrl(source.segment.media_url),
					isActive: () => source.audio !== null,
					onFatal: () => retrySource(),
				}).then((result) => {
					if (!source.audio) return;
					source.hls = result.hls;
				});
			}
		} else {
			source.audio.src = absoluteMediaUrl(source.segment.media_url);
		}
	}

	private updateLiveSources(position: number, context: AudioContext): void {
		for (const source of this.sources.values()) {
			if (source.audio) source.shouldPlay = false;
		}
		const activeSegmentIds = new Set<string>();
		for (const track of this.tracks) {
			let selected: ChannelMixSourceSegment | null = null;
			for (const segment of track.segments) {
				if (
					position < segment.start_ms ||
					position >= segment.end_ms ||
					this.sourceFor(segment, track.user_id).failed
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
		for (const track of this.tracks) {
			for (const segment of track.segments) {
				const source = this.sourceFor(segment, track.user_id);
				if (
					source.failed ||
					segment.end_ms <= position ||
					segment.start_ms > position + SCHEDULE_AHEAD_MS
				)
					continue;
				this.attachLiveSource(source, context);
				const activeNow = activeSegmentIds.has(segment.id);
				source.shouldPlay = activeNow;
				if (!activeNow || !source.audio) continue;
				active.add(segment.id);
				if (source.audio.readyState >= HTMLMediaElement.HAVE_FUTURE_DATA) {
					const desiredSeconds = this.liveDesiredSeconds(source, position);
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
					if (source.audio.paused) this.startLiveSource(source, position);
				}
			}
		}
		for (const source of this.sources.values()) {
			if (!source.audio || active.has(source.segment.id)) continue;
			source.audio.pause();
		}
	}
}
