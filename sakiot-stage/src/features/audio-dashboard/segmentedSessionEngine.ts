import type Hls from "hls.js";
import { absoluteMediaUrl } from "../../api/routes";
import {
	refreshForMediaRetry,
	SESSION_EXPIRED_MESSAGE,
} from "../../app/authedFetch";
import {
	attachHlsAudio,
	LIVE_EDGE_MARGIN_SECONDS,
	prefersNativeHls,
} from "../../shared/attachHls";
import {
	clampPlaybackPosition,
	isSameMediaSegment,
	isSameTimelineSegment,
	segmentAtPosition,
	shouldRetryMediaLoad,
} from "./logicalSessionPlaybackState";
import type { SessionSelection } from "./logicalSessionSelection";
import type { PlaybackSegment } from "./logicalSessionTimeline";
import { PlaybackBound } from "./playbackBound";
import { PlaybackStore } from "./playbackStore";

export interface SegmentedSessionOptions {
	segments: PlaybackSegment[];
	durationMs: number;
	volume: number;
	playbackRate: number;
	onError: (message: string | null) => void;
	onLoopDisabled: () => void;
}

export interface SegmentedSessionSnapshot {
	readonly positionMs: number;
	readonly seekPreviewMs: number | null;
	readonly playing: boolean;
	readonly boundActive: boolean;
}

/**
 * Plays a logical session as one timeline over its segments: recorded
 * fragments through a media element (HLS while one is still live) and gaps as
 * silence advanced on the wall clock. Each start bumps a generation so events
 * from a replaced source are ignored.
 *
 * The engine owns every piece of mutable playback state; React reads it
 * through the snapshot. `dispose` stops the source but leaves the engine
 * usable, which Strict Mode's mount, unmount, mount sequence needs.
 */
export class SegmentedSessionEngine extends PlaybackStore<SegmentedSessionSnapshot> {
	private segments: PlaybackSegment[];
	private durationMs: number;
	private volume: number;
	private rate: number;
	private onError: (message: string | null) => void;
	private onLoopDisabled: () => void;
	private audio: HTMLAudioElement | null = null;
	private hls: Hls | null = null;
	private animation: number | null = null;
	private generation = 0;
	private mediaRetry = false;
	private activeSegment: PlaybackSegment | null = null;
	/**
	 * A live fragment whose media ended before the manifest said where it
	 * ends; playback continues from that end once the manifest catches up.
	 */
	private endedLive: { generation: number; segment: PlaybackSegment } | null =
		null;
	private readonly bound: PlaybackBound;

	constructor(options: SegmentedSessionOptions) {
		super({
			positionMs: 0,
			seekPreviewMs: null,
			playing: false,
			boundActive: false,
		});
		this.segments = options.segments;
		this.durationMs = options.durationMs;
		this.volume = options.volume;
		this.rate = options.playbackRate;
		this.onError = options.onError;
		this.onLoopDisabled = options.onLoopDisabled;
		this.bound = new PlaybackBound({
			position: () => this.snapshot.positionMs,
			isPlaying: () => this.snapshot.playing,
			duration: () => this.durationMs,
			startAt: (positionMs, autoplay) => this.startAt(positionMs, autoplay),
			stop: () => this.stop(),
			loopDisabled: () => this.onLoopDisabled(),
			beforePlay: () => this.onError(null),
			clearSeekPreview: () => this.publish({ seekPreviewMs: null }),
			activeChanged: (boundActive) => this.publish({ boundActive }),
		});
	}

	setCallbacks(
		onError: (message: string | null) => void,
		onLoopDisabled: () => void,
	): void {
		this.onError = onError;
		this.onLoopDisabled = onLoopDisabled;
	}

	/**
	 * Takes a new manifest revision. While a session records, every revision
	 * extends its live fragment or trailing silence; the playing source reads
	 * its end from here rather than from when it started.
	 */
	setTimeline(segments: PlaybackSegment[], durationMs: number): void {
		this.segments = segments;
		this.durationMs = durationMs;
		const { positionMs, seekPreviewMs, playing } = this.snapshot;
		this.publish({
			// A live fragment plays slightly past the end the manifest last
			// reported; the next revision overtakes it.
			positionMs: playing ? positionMs : Math.min(positionMs, durationMs),
			seekPreviewMs:
				seekPreviewMs === null ? null : Math.min(seekPreviewMs, durationMs),
		});
		const ended = this.endedLive;
		if (ended && ended.generation === this.generation) {
			const end = this.segmentEnd(ended.segment);
			if (Number.isFinite(end)) {
				this.endedLive = null;
				this.continueAfter(end);
			}
		}
	}

	setPlaybackRate(rate: number): void {
		this.rate = rate;
		if (this.audio) this.audio.playbackRate = rate;
	}

	setVolume(volume: number): void {
		this.volume = volume;
		if (this.audio) this.audio.volume = volume;
	}

	readonly setSeekPreviewMs = (seekPreviewMs: number | null): void => {
		this.publish({ seekPreviewMs });
	};

	readonly togglePlay = (selection: SessionSelection, loop: boolean): void =>
		this.bound.togglePlay(selection, loop);

	readonly togglePreview = (selection: SessionSelection, loop: boolean): void =>
		this.bound.togglePreview(selection, loop);

	readonly updateLoop = (enabled: boolean, selection: SessionSelection): void =>
		this.bound.updateLoop(enabled, selection);

	readonly syncBound = (selection: SessionSelection, loop: boolean): void =>
		this.bound.sync(selection, loop);

	readonly clearBound = (): void => this.bound.clear();

	readonly stop = (): void => {
		this.bound.clear();
		this.generation += 1;
		this.stopSource();
		this.publish({ playing: false });
	};

	readonly seek = (
		nextPositionMs: number,
		selection: SessionSelection,
		loop: boolean,
	): void => {
		const target = this.liveEdgeAware(
			clampPlaybackPosition(nextPositionMs, this.durationMs),
		);
		this.bound.prepareSeek(selection, loop, target);
		this.seekWithinSource(target);
	};

	readonly restartWithSegments = (segments: PlaybackSegment[]): void => {
		this.segments = segments;
		this.startAt(this.snapshot.positionMs, this.snapshot.playing);
	};

	/** Stops the source; the engine stays reusable. */
	dispose(): void {
		this.generation += 1;
		this.stopSource();
		this.publish({ playing: false });
	}

	/**
	 * Where `segment` ends according to the latest manifest. A live fragment
	 * has no end yet: its media element waits at the live edge and ends when
	 * the recording finalizes the fragment.
	 */
	private segmentEnd(segment: PlaybackSegment): number {
		const latest =
			this.segments.find((candidate) =>
				isSameTimelineSegment(candidate, segment),
			) ?? segment;
		if (latest.kind === "active_hls") return Number.POSITIVE_INFINITY;
		return Math.min(latest.end_ms, this.durationMs);
	}

	/**
	 * A seek to the end of a session that is still recording lands inside its
	 * live fragment, whose media holds it behind the playlist's end, rather
	 * than past the last segment, where playback would stop.
	 */
	private liveEdgeAware(positionMs: number): number {
		const last = this.segments.at(-1);
		if (last?.kind !== "active_hls" || positionMs < last.end_ms) {
			return positionMs;
		}
		return Math.max(last.start_ms, last.end_ms - 1);
	}

	/** Moves on from the end of a segment: the bound, the next one, or stop. */
	private continueAfter(endMs: number): void {
		if (!this.bound.apply(endMs)) this.startAt(endMs, endMs < this.durationMs);
	}

	readonly startAt = (requestedPosition: number, autoplay: boolean): void => {
		this.generation += 1;
		this.endedLive = null;
		const generation = this.generation;
		this.stopSource();
		const durationMs = this.durationMs;
		const position = clampPlaybackPosition(requestedPosition, durationMs);
		this.publish({ positionMs: position, seekPreviewMs: null });
		if (!autoplay || position >= durationMs) {
			this.publish({ playing: false });
			return;
		}
		const segment = segmentAtPosition(this.segments, position);
		if (!segment) {
			this.activeSegment = null;
			this.publish({ playing: false });
			this.bound.clear();
			return;
		}
		this.activeSegment = segment;
		this.publish({ playing: true });
		if (segment.kind === "silence") {
			const wallStart = performance.now();
			const logicalStart = position;
			const tick = (wallNow: number) => {
				if (this.generation !== generation || !this.snapshot.playing) return;
				const next = logicalStart + (wallNow - wallStart) * this.rate;
				if (this.bound.apply(next)) return;
				const end = this.segmentEnd(segment);
				if (next >= end) {
					this.startAt(end, end < this.durationMs);
					return;
				}
				this.publish({ positionMs: next });
				this.animation = requestAnimationFrame(tick);
			};
			this.animation = requestAnimationFrame(tick);
			return;
		}
		const mediaUrl = segment.media_url;
		if (!mediaUrl) {
			this.continueAfter(Math.min(segment.end_ms, durationMs));
			return;
		}
		const audio = new Audio();
		audio.crossOrigin = "use-credentials";
		audio.preload = "auto";
		audio.volume = this.volume;
		audio.playbackRate = this.rate;
		this.audio = audio;
		const failSegment = (message: string) => {
			this.onError(message);
			this.publish({ playing: false });
			this.bound.clear();
		};
		const begin = (startSeconds?: number) => {
			if (this.generation !== generation) return;
			this.mediaRetry = false;
			const localSeconds = playableSeconds(
				audio,
				segment,
				startSeconds ?? Math.max(0, (position - segment.start_ms) / 1_000),
			);
			audio.currentTime = localSeconds;
			// Near the end of a live fragment playback starts earlier than
			// asked; the playhead shows where.
			this.publish({ positionMs: segment.start_ms + localSeconds * 1_000 });
			void audio.play().catch((error: unknown) => {
				if (this.generation !== generation) return;
				if (error instanceof DOMException && error.name === "AbortError")
					return;
				failSegment("Browser blocked or failed audio playback.");
			});
		};
		const retryOrFail = () => {
			const generationMatches = this.generation === generation;
			if (!shouldRetryMediaLoad(this.mediaRetry, generationMatches)) {
				if (generationMatches) {
					failSegment(`Could not load segment ${segment.segment_index ?? ""}.`);
				}
				return;
			}
			this.mediaRetry = true;
			void refreshForMediaRetry().then((ok) => {
				if (this.generation !== generation) return;
				if (ok) {
					this.onError(null);
					this.startAt(this.snapshot.positionMs, true);
				} else {
					failSegment(SESSION_EXPIRED_MESSAGE);
				}
			});
		};
		audio.addEventListener("pause", () => {
			if (this.generation !== generation) return;
			this.publish({ playing: false });
		});
		// `timeupdate` only fires a few times a second, so a playhead driven
		// by it alone steps over audio while the silence branches, advanced
		// per animation frame, glide. Read the media clock every frame and
		// keep `timeupdate` as the fallback for throttled or hidden tabs.
		const syncFromAudio = () => {
			if (this.generation !== generation) return false;
			const mediaSeconds = audio.currentTime;
			if (!Number.isFinite(mediaSeconds)) return true;
			const logical = segment.start_ms + mediaSeconds * 1_000;
			if (this.bound.apply(logical)) return false;
			const end = this.segmentEnd(segment);
			if (logical >= end - 20) {
				this.continueAfter(end);
				return false;
			}
			this.publish({ positionMs: logical });
			return true;
		};
		const tickFromAudio = () => {
			this.animation = null;
			if (this.generation !== generation || !this.snapshot.playing) return;
			if (!syncFromAudio()) return;
			this.animation = requestAnimationFrame(tickFromAudio);
		};
		const startAudioTicker = () => {
			if (this.generation !== generation) return;
			if (this.animation !== null) return;
			this.animation = requestAnimationFrame(tickFromAudio);
		};
		audio.addEventListener("playing", startAudioTicker);
		audio.addEventListener("timeupdate", () => {
			if (this.generation !== generation) return;
			syncFromAudio();
		});
		audio.addEventListener("ended", () => {
			if (this.generation !== generation) return;
			const end = this.segmentEnd(segment);
			if (Number.isFinite(end)) {
				this.continueAfter(end);
			} else {
				// The recording finalized the fragment before the manifest
				// poll noticed: wait for the revision that says where it ended.
				this.endedLive = { generation, segment };
			}
		});
		audio.addEventListener("error", retryOrFail);
		if (segment.kind === "active_hls" && segment.hls_playlist_url) {
			const hlsUrl = absoluteMediaUrl(segment.hls_playlist_url);
			if (prefersNativeHls(audio)) {
				audio.src = hlsUrl;
				audio.addEventListener("loadedmetadata", () => begin(), {
					once: true,
				});
			} else {
				void attachHlsAudio({
					audio,
					playlistUrl: hlsUrl,
					fallbackUrl: absoluteMediaUrl(mediaUrl),
					isActive: () => this.generation === generation,
					onFatal: () => retryOrFail(),
					onManifestParsed: begin,
					unlimitedMaxLatency: true,
					// Without it hls.js starts a live playlist at its live
					// edge and seeks there after `begin`, so the playhead
					// jumps away from where playback was asked to start.
					startPositionSeconds: Math.max(
						0,
						(position - segment.start_ms) / 1_000,
					),
				}).then((result) => {
					if (this.generation !== generation) return;
					if (result.kind === "direct") {
						audio.addEventListener("loadedmetadata", () => begin(), {
							once: true,
						});
					}
					this.hls = result.hls;
				});
			}
		} else {
			audio.src = absoluteMediaUrl(mediaUrl);
			audio.addEventListener("loadedmetadata", () => begin(), { once: true });
		}
	};

	private stopSource(): void {
		if (this.animation !== null) {
			cancelAnimationFrame(this.animation);
			this.animation = null;
		}
		this.hls?.destroy();
		this.hls = null;
		if (this.audio) {
			this.audio.pause();
			this.audio.removeAttribute("src");
			this.audio.load();
			this.audio = null;
		}
	}

	private seekWithinSource(position: number): void {
		const targetSegment = segmentAtPosition(this.segments, position);
		const audio = this.audio;
		if (
			this.snapshot.playing &&
			audio &&
			isSameMediaSegment(this.activeSegment, targetSegment)
		) {
			try {
				const segmentStart = targetSegment?.start_ms ?? 0;
				const localSeconds = targetSegment
					? playableSeconds(
							audio,
							targetSegment,
							Math.max(0, (position - segmentStart) / 1_000),
						)
					: 0;
				audio.currentTime = localSeconds;
				this.publish({ positionMs: segmentStart + localSeconds * 1_000 });
				return;
			} catch {
				// Replacing the source below handles media that cannot seek in place.
			}
		}
		this.startAt(position, this.snapshot.playing);
	}
}

/**
 * Where a fragment's media can play from, near `seconds`. A live fragment's
 * duration is the end of its playlist, which is still being written: stay
 * `LIVE_EDGE_MARGIN_SECONDS` behind it rather than stall at the very end.
 */
function playableSeconds(
	audio: HTMLAudioElement,
	segment: PlaybackSegment,
	seconds: number,
): number {
	if (!Number.isFinite(audio.duration)) return seconds;
	const margin =
		segment.kind === "active_hls" ? LIVE_EDGE_MARGIN_SECONDS : 0.01;
	return Math.min(seconds, Math.max(0, audio.duration - margin));
}
