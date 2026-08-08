import type { ClipEdit, SegmentEffects, TimelineSegment } from "./model";
import { editDuration, effectiveRate, segmentDuration } from "./model";

function dbToLinear(db: number): number {
	return 10 ** (db / 20);
}

/**
 * Source-buffer window a segment must play to fill the given timeline range.
 * AudioBufferSourceNode.start() measures its duration in buffer seconds and
 * consumes the buffer at the computed rate (playbackRate times the detune
 * pitch factor), so a timeline window has to be scaled by that effective
 * rate: a 2x segment consumes twice the buffer content per timeline second,
 * and a pitch shift likewise accelerates or slows the consumption. Without
 * the scaling, fast clips go silent halfway through their box and slow clips
 * keep playing past its end.
 */
export function segmentSourceWindow(
	segment: TimelineSegment,
	fromSec: number,
	overlapStart: number,
	overlapEnd: number,
): { offset: number; duration: number } {
	const rate = effectiveRate(segment.effects);
	return {
		offset:
			segment.sourceIn + Math.max(0, fromSec - segment.timelineStart) * rate,
		duration: (overlapEnd - overlapStart) * rate,
	};
}

interface ActiveSource {
	node: AudioBufferSourceNode;
	gain: GainNode;
	bass: BiquadFilterNode;
	treble: BiquadFilterNode;
}

/**
 * One AudioContext rendering a ClipEdit. Segments are scheduled as
 * AudioBufferSourceNodes; the same graph shape is used by the offline
 * renderer later, so what you hear is what will export.
 */
export class ClipEditorEngine {
	private ctx: AudioContext | null = null;
	private bass: BiquadFilterNode | null = null;
	private treble: BiquadFilterNode | null = null;
	private master: GainNode | null = null;
	private active = new Map<string, ActiveSource>();
	private endTimer: number | null = null;
	private startedAtMs = 0;
	private playheadSec = 0;
	private lastPlay: {
		edit: ClipEdit;
		buffers: ReadonlyMap<string, AudioBuffer>;
		loop: boolean;
	} | null = null;

	get isPlaying(): boolean {
		return this.active.size > 0 || this.endTimer !== null;
	}

	get positionSec(): number {
		if (!this.isPlaying) return this.playheadSec;
		return this.playheadSec + (performance.now() - this.startedAtMs) / 1000;
	}

	play(
		edit: ClipEdit,
		fromSec: number,
		buffers: ReadonlyMap<string, AudioBuffer>,
		loop: boolean,
	) {
		this.cancel();
		this.lastPlay = { edit, buffers, loop };
		this.playheadSec = fromSec;
		this.startedAtMs = performance.now();
		const ctx = this.ensureContext();
		if (this.master) this.master.gain.value = dbToLinear(edit.masterVolumeDb);
		const now = ctx.currentTime;
		const totalDuration = editDuration(edit);
		let lastEndMs = 0;
		for (const segment of edit.segments) {
			const buffer = buffers.get(segment.sourceId);
			if (!buffer) continue;
			const duration = segmentDuration(segment);
			if (duration <= 0) continue;
			const segmentEndSec = segment.timelineStart + duration;
			const overlapStart = Math.max(fromSec, segment.timelineStart);
			const overlapEnd = Math.min(fromSec + totalDuration, segmentEndSec);
			if (overlapEnd <= overlapStart) continue;
			const node = ctx.createBufferSource();
			node.buffer = buffer;
			node.detune.value = segment.effects.pitchCents;
			node.playbackRate.value = segment.effects.rate;
			const gain = ctx.createGain();
			gain.gain.value = dbToLinear(segment.effects.volumeDb);
			const bass = ctx.createBiquadFilter();
			bass.type = "lowshelf";
			bass.frequency.value = 250;
			bass.gain.value = segment.effects.bassDb;
			const treble = ctx.createBiquadFilter();
			treble.type = "highshelf";
			treble.frequency.value = 3000;
			treble.gain.value = segment.effects.trebleDb;
			node.connect(gain);
			gain.connect(bass);
			bass.connect(treble);
			treble.connect(this.bass ?? ctx.destination);
			const sourceWindow = segmentSourceWindow(
				segment,
				fromSec,
				overlapStart,
				overlapEnd,
			);
			node.start(
				now + Math.max(0, overlapStart - fromSec),
				sourceWindow.offset,
				sourceWindow.duration,
			);
			this.active.set(segment.id, { node, gain, bass, treble });
			lastEndMs = Math.max(
				lastEndMs,
				performance.now() + (overlapEnd - fromSec) * 1000,
			);
		}
		if (lastEndMs > 0) {
			this.endTimer = window.setTimeout(
				() => {
					this.endTimer = null;
					if (loop && this.lastPlay) {
						this.play(
							this.lastPlay.edit,
							0,
							this.lastPlay.buffers,
							this.lastPlay.loop,
						);
					} else {
						this.finish();
					}
				},
				Math.max(0, lastEndMs - performance.now()),
			);
		} else {
			this.finish();
		}
	}

	stop() {
		this.cancel();
		this.playheadSec = 0;
	}

	cancel() {
		if (this.endTimer !== null) {
			clearTimeout(this.endTimer);
			this.endTimer = null;
		}
		for (const { node } of this.active.values()) {
			try {
				node.stop();
			} catch {
				// Already stopped (loop restart or overlap resechedule).
			}
		}
		this.active.clear();
	}

	seekTo(
		edit: ClipEdit,
		sec: number,
		buffers: ReadonlyMap<string, AudioBuffer>,
		loop: boolean,
	) {
		if (this.isPlaying) {
			this.play(edit, sec, buffers, loop);
		} else {
			this.playheadSec = sec;
		}
	}

	applySegmentEffects(id: string, effects: SegmentEffects) {
		const active = this.active.get(id);
		if (!active) return;
		active.node.detune.value = effects.pitchCents;
		active.node.playbackRate.value = effects.rate;
		active.gain.gain.value = dbToLinear(effects.volumeDb);
		active.bass.gain.value = effects.bassDb;
		active.treble.gain.value = effects.trebleDb;
	}

	setMasterVolume(db: number) {
		if (this.master) this.master.gain.value = dbToLinear(db);
	}

	dispose() {
		this.cancel();
		if (this.ctx) {
			void this.ctx.close();
			this.ctx = null;
		}
	}

	private finish() {
		this.active.clear();
		this.playheadSec = 0;
	}

	private ensureContext(): AudioContext {
		if (!this.ctx) {
			this.ctx = new AudioContext();
			this.bass = this.ctx.createBiquadFilter();
			this.bass.type = "lowshelf";
			this.bass.frequency.value = 250;
			this.treble = this.ctx.createBiquadFilter();
			this.treble.type = "highshelf";
			this.treble.frequency.value = 3000;
			this.master = this.ctx.createGain();
			this.bass.connect(this.treble);
			this.treble.connect(this.master);
			this.master.connect(this.ctx.destination);
		}
		if (this.ctx.state === "suspended") void this.ctx.resume();
		return this.ctx;
	}
}
