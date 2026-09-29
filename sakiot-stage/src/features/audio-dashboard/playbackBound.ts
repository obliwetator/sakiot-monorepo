import type { SessionSelection } from "./logicalSessionSelection";
import { selectionContainsPosition } from "./logicalSessionSelection";

interface BoundRange {
	stopMs: number;
	loopToMs: number | null;
}

/** What a bound needs from the playback engine that owns it. */
export interface BoundedPlayback {
	position(): number;
	isPlaying(): boolean;
	duration(): number;
	startAt(positionMs: number, autoplay: boolean): void;
	/** The engine's own stop, run when bound playback must end first. */
	stop(): void;
	loopDisabled(): void;
	/** Clears the engine's playback error before a bound starts. */
	beforePlay(): void;
	clearSeekPreview(): void;
	activeChanged(active: boolean): void;
}

/**
 * The loop/stop boundary both session playback engines share: which logical
 * position the current selection stops at or loops back to, plus the
 * operations that arm, apply, and re-sync it. Segmented and silence-free
 * playback once had byte-identical copies of this state machine.
 */
export class PlaybackBound {
	private range: BoundRange | null = null;

	constructor(private readonly playback: BoundedPlayback) {}

	clear(): void {
		this.range = null;
		this.playback.activeChanged(false);
	}

	/** Stops or loops at the bound; true when it moved the playhead. */
	apply(atMs: number): boolean {
		const range = this.range;
		if (!range || atMs < range.stopMs) return false;
		if (range.loopToMs !== null) {
			this.playback.startAt(range.loopToMs, true);
			return true;
		}
		this.clear();
		this.playback.startAt(range.stopMs, false);
		return true;
	}

	togglePlay(selection: SessionSelection, loop: boolean): void {
		if (this.playback.isPlaying()) {
			this.playback.stop();
			return;
		}
		this.playback.beforePlay();
		const current = this.playback.position();
		if (loop && selection[1] > selection[0]) {
			this.arm({ stopMs: selection[1], loopToMs: selection[0] });
			this.playback.startAt(
				selectionContainsPosition(selection, current) ? current : selection[0],
				true,
			);
			return;
		}
		this.clear();
		this.playback.startAt(
			current >= this.playback.duration() ? 0 : current,
			true,
		);
	}

	togglePreview(selection: SessionSelection, loop: boolean): void {
		if (this.range) {
			this.playback.stop();
			return;
		}
		if (selection[1] <= selection[0]) return;
		this.arm({ stopMs: selection[1], loopToMs: loop ? selection[0] : null });
		this.playback.clearSeekPreview();
		this.playback.startAt(selection[0], true);
	}

	updateLoop(enabled: boolean, selection: SessionSelection): void {
		if (!enabled) {
			if (this.range) this.range.loopToMs = null;
			return;
		}
		if (this.range) {
			this.arm({ stopMs: selection[1], loopToMs: selection[0] });
			this.playback.startAt(selection[0], this.playback.isPlaying());
			return;
		}
		this.clear();
		this.playback.startAt(selection[0], false);
	}

	sync(selection: SessionSelection, loop: boolean): void {
		if (!this.range) return;
		const position = this.playback.position();
		if (!selectionContainsPosition(selection, position)) {
			this.clear();
			if (loop) this.playback.loopDisabled();
			return;
		}
		this.range = { stopMs: selection[1], loopToMs: loop ? selection[0] : null };
		this.apply(position);
	}

	/** Re-arms the bound for a seek target; the caller moves the media. */
	prepareSeek(selection: SessionSelection, loop: boolean, targetMs: number) {
		if (loop && !selectionContainsPosition(selection, targetMs)) {
			this.clear();
			this.playback.loopDisabled();
		} else if (this.range) {
			this.range = {
				stopMs: selection[1],
				loopToMs: loop ? selection[0] : null,
			};
		}
	}

	private arm(range: BoundRange): void {
		this.range = range;
		this.playback.activeChanged(true);
	}
}
