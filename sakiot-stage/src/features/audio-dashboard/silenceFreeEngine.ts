import {
	refreshForMediaRetry,
	SESSION_EXPIRED_MESSAGE,
} from "../../app/authedFetch";
import { clampPlaybackPosition } from "./logicalSessionPlaybackState";
import type { SessionSelection } from "./logicalSessionSelection";
import { PlaybackBound } from "./playbackBound";
import { PlaybackStore } from "./playbackStore";

export interface SilenceFreeOptions {
	mediaUrl: string | null;
	initialDurationMs?: number;
	volume: number;
	playbackRate: number;
	onLoopDisabled: () => void;
}

export interface SilenceFreeSnapshot {
	readonly positionMs: number;
	readonly seekPreviewMs: number | null;
	readonly durationMs: number;
	readonly playing: boolean;
	readonly playbackError: string | null;
	/** Bumped to remount the `<audio>` element after a refreshed session. */
	readonly retryKey: number;
	readonly boundActive: boolean;
}

/**
 * Plays one generated, silence-free rendition through an `<audio>` element
 * that React renders: the element reports to `mediaHandlers`, and the engine
 * drives it through `audioRef`.
 *
 * The engine owns every piece of mutable playback state; React reads it
 * through the snapshot. `dispose` stops the frame loop but leaves the engine
 * usable, which Strict Mode's mount, unmount, mount sequence needs.
 */
export class SilenceFreeEngine extends PlaybackStore<SilenceFreeSnapshot> {
	/** Handed to the rendered `<audio>` element as its ref. */
	readonly audioRef: { current: HTMLAudioElement | null } = { current: null };
	private mediaUrl: string | null;
	private initialDurationMs: number | undefined;
	private volume: number;
	private rate: number;
	private onLoopDisabled: () => void;
	private retried = false;
	private frame: number | null = null;
	private readonly bound: PlaybackBound;

	constructor(options: SilenceFreeOptions) {
		super({
			positionMs: 0,
			seekPreviewMs: null,
			durationMs: options.initialDurationMs ?? 0,
			playing: false,
			playbackError: null,
			retryKey: 0,
			boundActive: false,
		});
		this.mediaUrl = options.mediaUrl;
		this.initialDurationMs = options.initialDurationMs;
		this.volume = options.volume;
		this.rate = options.playbackRate;
		this.onLoopDisabled = options.onLoopDisabled;
		this.bound = new PlaybackBound({
			position: () => this.snapshot.positionMs,
			isPlaying: () => this.snapshot.playing,
			duration: () => this.snapshot.durationMs,
			startAt: (positionMs, autoplay) => this.startAt(positionMs, autoplay),
			stop: () => this.stop(),
			loopDisabled: () => this.onLoopDisabled(),
			beforePlay: () => this.publish({ playbackError: null }),
			clearSeekPreview: () => this.publish({ seekPreviewMs: null }),
			activeChanged: (boundActive) => this.publish({ boundActive }),
		});
	}

	/**
	 * Each generated media resource starts with its own duration, position,
	 * and single-retry state.
	 */
	setMedia(mediaUrl: string | null, initialDurationMs: number | undefined) {
		if (
			mediaUrl === this.mediaUrl &&
			initialDurationMs === this.initialDurationMs
		) {
			return;
		}
		this.mediaUrl = mediaUrl;
		this.initialDurationMs = initialDurationMs;
		this.retried = false;
		this.publish({
			durationMs: initialDurationMs ?? 0,
			positionMs: 0,
			seekPreviewMs: null,
			playbackError: null,
		});
		this.bound.clear();
	}

	setLoopDisabledListener(listener: () => void): void {
		this.onLoopDisabled = listener;
	}

	setVolume(volume: number): void {
		this.volume = volume;
		if (this.audioRef.current) this.audioRef.current.volume = volume;
	}

	setPlaybackRate(rate: number): void {
		this.rate = rate;
		if (this.audioRef.current) this.audioRef.current.playbackRate = rate;
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
		this.audioRef.current?.pause();
		this.setPlaying(false);
	};

	readonly startAt = (requestedPosition: number, autoplay: boolean): void => {
		const audio = this.audioRef.current;
		const durationMs = this.snapshot.durationMs;
		const position = clampPlaybackPosition(requestedPosition, durationMs);
		this.publish({ positionMs: position, seekPreviewMs: null });
		if (!audio) return;
		try {
			audio.currentTime = position / 1_000;
		} catch {
			this.publish({
				playbackError: "Silence-free audio could not be seeked.",
			});
			return;
		}
		if (!autoplay || position >= durationMs) {
			audio.pause();
			this.setPlaying(false);
			return;
		}
		this.publish({ playbackError: null });
		void audio.play().catch(() => {
			this.setPlaying(false);
			this.bound.clear();
			this.publish({
				playbackError: "Browser blocked or failed silence-free playback.",
			});
		});
	};

	readonly seek = (
		nextPositionMs: number,
		selection: SessionSelection,
		loop: boolean,
	): void => {
		this.publish({ seekPreviewMs: null });
		const target = clampPlaybackPosition(
			nextPositionMs,
			this.snapshot.durationMs,
		);
		this.bound.prepareSeek(selection, loop, target);
		const audio = this.audioRef.current;
		if (audio) {
			try {
				audio.currentTime = target / 1_000;
			} catch {
				this.publish({
					playbackError: "Silence-free audio could not be seeked.",
				});
				return;
			}
		}
		this.publish({ positionMs: target });
		this.bound.apply(target);
	};

	/** Event handlers for the rendered `<audio>` element. */
	readonly mediaHandlers = {
		onLoadedMetadata: (audio: HTMLAudioElement) => {
			this.retried = false;
			this.updateDuration(audio);
		},
		onDurationChange: (audio: HTMLAudioElement) => this.updateDuration(audio),
		onTimeUpdate: (audio: HTMLAudioElement) => this.timeUpdate(audio),
		onPlay: () => this.setPlaying(true),
		onPause: () => this.setPlaying(false),
		onEnded: () => {
			this.setPlaying(false);
			this.bound.clear();
		},
		onError: () => this.handleError(),
	};

	/** Stops the frame loop; the engine stays reusable. */
	dispose(): void {
		this.stopFrameLoop();
	}

	private updateDuration(audio: HTMLAudioElement): void {
		if (!Number.isFinite(audio.duration) || audio.duration <= 0) return;
		this.publish({ durationMs: audio.duration * 1_000 });
		audio.volume = this.volume;
		audio.playbackRate = this.rate;
	}

	private timeUpdate(audio: HTMLAudioElement): void {
		const next = audio.currentTime * 1_000;
		if (this.bound.apply(next)) return;
		this.publish({ positionMs: next });
	}

	private handleError(): void {
		this.setPlaying(false);
		this.bound.clear();
		if (this.retried) {
			this.publish({
				playbackError: "Silence-free audio could not be loaded.",
			});
			return;
		}
		this.retried = true;
		void refreshForMediaRetry().then((ok) => {
			if (ok) {
				this.publish({
					playbackError: null,
					retryKey: this.snapshot.retryKey + 1,
				});
			} else {
				this.publish({ playbackError: SESSION_EXPIRED_MESSAGE });
			}
		});
	}

	private setPlaying(playing: boolean): void {
		this.publish({ playing });
		if (playing) {
			this.startFrameLoop();
		} else {
			this.stopFrameLoop();
		}
	}

	/**
	 * `timeupdate` fires only a few times a second, so while the element plays
	 * the playhead follows its clock every animation frame instead.
	 */
	private startFrameLoop(): void {
		if (this.frame !== null) return;
		const update = () => {
			this.frame = null;
			const audio = this.audioRef.current;
			if (!audio || audio.paused || !this.snapshot.playing) return;
			this.timeUpdate(audio);
			this.frame = requestAnimationFrame(update);
		};
		this.frame = requestAnimationFrame(update);
	}

	private stopFrameLoop(): void {
		if (this.frame === null) return;
		cancelAnimationFrame(this.frame);
		this.frame = null;
	}
}
