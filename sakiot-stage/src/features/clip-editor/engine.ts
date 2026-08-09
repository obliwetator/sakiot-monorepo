import {
	connect as connectTone,
	Filter,
	PitchShift,
	Context as ToneContext,
	Volume,
} from "tone";
import { PARITY_APPROVED_EQ } from "./effectParity";
import type { ClipEdit, SegmentEffects, TimelineSegment } from "./model";
import {
	editDuration,
	effectiveRate,
	segmentDuration,
	sourcePositionAt,
} from "./model";

/**
 * Tone shifts the stream after playbackRate has already repitched it. Remove
 * that rate-induced pitch first, then apply the user's pitch in semitones.
 */
export function pitchShiftSemitones(effects: SegmentEffects): number {
	return (
		effects.pitchCents / 100 - 12 * Math.log2(Math.max(0.01, effects.rate))
	);
}

/**
 * Source-buffer window a segment must play to fill the given timeline range.
 * AudioBufferSourceNode.start() measures its duration in buffer seconds and
 * consumes the buffer at playbackRate, so a timeline window has to be scaled
 * by that rate: a 2x segment consumes twice the buffer content per timeline
 * second. Pitch shifting happens downstream and preserves this duration.
 * Without the scaling, fast clips go silent halfway through their box and
 * slow clips keep playing past its end. Reversed segments start at the
 * source-window end and walk backwards at the same rate (negative rate).
 */
export function segmentSourceWindow(
	segment: TimelineSegment,
	// The offset is fully determined by the overlap window; kept in the
	// signature for callers that compute it alongside the overlap.
	_fromSec: number,
	overlapStart: number,
	overlapEnd: number,
): { offset: number; duration: number } {
	const rate = effectiveRate(segment.effects);
	return {
		offset: sourcePositionAt(segment, overlapStart),
		duration: (overlapEnd - overlapStart) * rate,
	};
}

interface SegmentAudioGraph {
	connectSource(source: AudioBufferSourceNode): void;
	setEffects(effects: SegmentEffects): void;
	dispose(): void;
}

interface EditorAudioGraph {
	createSegment(effects: SegmentEffects): SegmentAudioGraph;
	setMasterVolume(db: number): void;
	dispose(): void;
}

export type EditorAudioGraphFactory = (ctx: AudioContext) => EditorAudioGraph;

class ToneEditorAudioGraph implements EditorAudioGraph {
	private readonly context: ToneContext;
	private readonly master: Volume;

	constructor(ctx: AudioContext) {
		this.context = new ToneContext({ context: ctx, lookAhead: 0 });
		this.master = new Volume({ context: this.context, volume: 0 });
		this.master.connect(ctx.destination);
	}

	createSegment(effects: SegmentEffects): SegmentAudioGraph {
		const pitchSemitones = pitchShiftSemitones(effects);
		const pitch =
			Math.abs(pitchSemitones) > 0.000_001
				? new PitchShift({
						context: this.context,
						pitch: pitchSemitones,
						windowSize: 0.05,
						wet: 1,
					})
				: null;
		const volume = new Volume({
			context: this.context,
			volume: effects.volumeDb,
		});
		const bass = new Filter({
			context: this.context,
			frequency: PARITY_APPROVED_EQ.bass.frequencyHz,
			gain: effects.bassDb,
			rolloff: -12,
			type: PARITY_APPROVED_EQ.bass.toneType,
		});
		const mid = new Filter({
			context: this.context,
			frequency: PARITY_APPROVED_EQ.mid.frequencyHz,
			gain: effects.midDb,
			Q: PARITY_APPROVED_EQ.mid.width,
			rolloff: -12,
			type: PARITY_APPROVED_EQ.mid.toneType,
		});
		const treble = new Filter({
			context: this.context,
			frequency: PARITY_APPROVED_EQ.treble.frequencyHz,
			gain: effects.trebleDb,
			rolloff: -12,
			type: PARITY_APPROVED_EQ.treble.toneType,
		});
		volume.chain(bass, mid, treble, this.master);

		return {
			connectSource(source) {
				if (pitch) {
					connectTone(source, pitch);
					pitch.connect(volume);
				} else {
					connectTone(source, volume);
				}
			},
			setEffects(next) {
				if (pitch) pitch.pitch = pitchShiftSemitones(next);
				volume.volume.value = next.volumeDb;
				bass.gain.value = next.bassDb;
				mid.gain.value = next.midDb;
				treble.gain.value = next.trebleDb;
			},
			dispose() {
				pitch?.dispose();
				volume.dispose();
				bass.dispose();
				mid.dispose();
				treble.dispose();
			},
		};
	}

	setMasterVolume(db: number) {
		this.master.volume.value = db;
	}

	dispose() {
		this.master.dispose();
		this.context.dispose();
	}
}

interface ActiveSource {
	node: AudioBufferSourceNode;
	graph: SegmentAudioGraph;
}

/**
 * One AudioContext rendering a ClipEdit. Segments are scheduled as
 * AudioBufferSourceNodes; the server renderer uses the same effect semantics
 * when it exports the composition.
 */
export class ClipEditorEngine {
	private ctx: AudioContext | null = null;
	private audioGraph: EditorAudioGraph | null = null;
	private active = new Map<string, ActiveSource>();
	private endTimer: number | null = null;
	private startedAtMs = 0;
	private playheadSec = 0;
	private lastPlay: {
		edit: ClipEdit;
		buffers: ReadonlyMap<string, AudioBuffer>;
		loop: boolean;
	} | null = null;

	constructor(
		private readonly createAudioGraph: EditorAudioGraphFactory = (ctx) =>
			new ToneEditorAudioGraph(ctx),
	) {}

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
		this.audioGraph?.setMasterVolume(edit.masterVolumeDb);
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
			node.detune.value = 0;
			node.playbackRate.value = segment.effects.reverse
				? -segment.effects.rate
				: segment.effects.rate;
			const graph = this.audioGraph?.createSegment(segment.effects);
			if (!graph) continue;
			graph.connectSource(node);
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
			this.active.set(segment.id, { node, graph });
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

	/** Pause playback, keeping the playhead where it is so play() resumes. */
	pause() {
		this.playheadSec = this.positionSec;
		this.cancel();
	}

	cancel() {
		if (this.endTimer !== null) {
			clearTimeout(this.endTimer);
			this.endTimer = null;
		}
		for (const { node, graph } of this.active.values()) {
			try {
				node.stop();
			} catch {
				// Already stopped (loop restart or overlap resechedule).
			}
			graph.dispose();
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
		active.node.detune.value = 0;
		active.node.playbackRate.value = effects.reverse
			? -effects.rate
			: effects.rate;
		active.graph.setEffects(effects);
	}

	setMasterVolume(db: number) {
		this.audioGraph?.setMasterVolume(db);
	}

	dispose() {
		this.cancel();
		this.audioGraph?.dispose();
		this.audioGraph = null;
		this.ctx = null;
	}

	private finish() {
		for (const { graph } of this.active.values()) graph.dispose();
		this.active.clear();
		this.playheadSec = 0;
	}

	private ensureContext(): AudioContext {
		if (!this.ctx) {
			this.ctx = new AudioContext();
			this.audioGraph = this.createAudioGraph(this.ctx);
		}
		if (this.ctx.state === "suspended") void this.ctx.resume();
		return this.ctx;
	}
}
