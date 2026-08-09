import { PARITY_APPROVED_EQ } from "./effectParity";
import type { ClipEdit, SegmentEffects, TimelineSegment } from "./model";
import {
	editDuration,
	effectiveRate,
	segmentContentDuration,
	segmentDuration,
	sourcePositionAt,
} from "./model";
import {
	renderSharedSegment,
	sharedDspSettled,
	warmSharedDsp,
} from "./sharedDsp";

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
	const contentEnd = segment.timelineStart + segmentContentDuration(segment);
	return {
		offset: sourcePositionAt(segment, overlapStart),
		duration:
			Math.max(0, Math.min(overlapEnd, contentEnd) - overlapStart) * rate,
	};
}

interface SegmentAudioGraph {
	connectSource(source: AudioBufferSourceNode): void;
	setEffects(effects: SegmentEffects): void;
	dispose(): void;
}

interface EditorAudioGraph {
	createSegment(
		effects: SegmentEffects,
		preprocessed?: boolean,
	): SegmentAudioGraph;
	setMasterVolume(db: number): void;
	dispose(): void;
}

export type EditorAudioGraphFactory = (ctx: AudioContext) => EditorAudioGraph;

class NativeEditorAudioGraph implements EditorAudioGraph {
	private readonly master: GainNode;

	constructor(ctx: AudioContext) {
		this.master = ctx.createGain();
		this.master.connect(ctx.destination);
	}

	createSegment(
		effects: SegmentEffects,
		preprocessed = false,
	): SegmentAudioGraph {
		if (preprocessed) {
			return {
				connectSource: (source) => source.connect(this.master),
				setEffects: () => {},
				dispose: () => {},
			};
		}
		const context = this.master.context;
		const volume = context.createGain();
		const bass = context.createBiquadFilter();
		const mid = context.createBiquadFilter();
		const treble = context.createBiquadFilter();
		bass.type = PARITY_APPROVED_EQ.bass.webAudioType;
		bass.frequency.value = PARITY_APPROVED_EQ.bass.frequencyHz;
		mid.type = PARITY_APPROVED_EQ.mid.webAudioType;
		mid.frequency.value = PARITY_APPROVED_EQ.mid.frequencyHz;
		mid.Q.value = PARITY_APPROVED_EQ.mid.width;
		treble.type = PARITY_APPROVED_EQ.treble.webAudioType;
		treble.frequency.value = PARITY_APPROVED_EQ.treble.frequencyHz;
		volume.connect(bass);
		bass.connect(mid);
		mid.connect(treble);
		treble.connect(this.master);

		const update = (next: SegmentEffects) => {
			volume.gain.value = dbToGain(next.volumeDb);
			bass.gain.value = next.bassDb;
			mid.gain.value = next.midDb;
			treble.gain.value = next.trebleDb;
		};
		update(effects);

		return {
			connectSource: (source) => source.connect(volume),
			setEffects: update,
			dispose() {
				volume.disconnect();
				bass.disconnect();
				mid.disconnect();
				treble.disconnect();
			},
		};
	}

	setMasterVolume(db: number) {
		this.master.gain.value = dbToGain(db);
	}

	dispose() {
		this.master.disconnect();
	}
}

function dbToGain(db: number): number {
	return 10 ** (db / 20);
}

interface ActiveSource {
	node: AudioBufferSourceNode;
	graph: SegmentAudioGraph;
	preprocessed: boolean;
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
	private pendingPlayback = false;
	private playbackGeneration = 0;
	private startedAtMs = 0;
	private playheadSec = 0;
	private lastPlay: {
		edit: ClipEdit;
		buffers: ReadonlyMap<string, AudioBuffer>;
		loop: boolean;
	} | null = null;

	constructor(
		private readonly createAudioGraph: EditorAudioGraphFactory = (ctx) =>
			new NativeEditorAudioGraph(ctx),
	) {
		void warmSharedDsp();
	}

	get isPlaying(): boolean {
		return (
			this.pendingPlayback || this.active.size > 0 || this.endTimer !== null
		);
	}

	get positionSec(): number {
		if (!this.isPlaying) return this.playheadSec;
		if (this.pendingPlayback) return this.playheadSec;
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
		const ctx = this.ensureContext();
		if (!sharedDspSettled()) {
			this.pendingPlayback = true;
			const generation = this.playbackGeneration;
			void warmSharedDsp().then(() => {
				if (generation !== this.playbackGeneration || !this.pendingPlayback)
					return;
				this.pendingPlayback = false;
				this.schedulePlayback(ctx, edit, fromSec, buffers, loop);
			});
			return;
		}
		this.schedulePlayback(ctx, edit, fromSec, buffers, loop);
	}

	private schedulePlayback(
		ctx: AudioContext,
		edit: ClipEdit,
		fromSec: number,
		buffers: ReadonlyMap<string, AudioBuffer>,
		loop: boolean,
	) {
		const totalDuration = editDuration(edit);
		const prepared: Array<{
			segment: TimelineSegment;
			buffer: AudioBuffer;
			rendered: AudioBuffer | null;
			overlapStart: number;
			overlapEnd: number;
		}> = [];
		for (const segment of edit.segments) {
			const buffer = buffers.get(segment.sourceId);
			if (!buffer) continue;
			const duration = segmentDuration(segment);
			if (duration <= 0) continue;
			const segmentEndSec = segment.timelineStart + duration;
			const overlapStart = Math.max(fromSec, segment.timelineStart);
			const overlapEnd = Math.min(fromSec + totalDuration, segmentEndSec);
			if (overlapEnd <= overlapStart) continue;
			prepared.push({
				segment,
				buffer,
				rendered: renderSharedSegment(ctx, buffer, segment),
				overlapStart,
				overlapEnd,
			});
		}

		// Rendering can be expensive. Establish the transport time only after all
		// buffers are ready so every segment is scheduled from the same origin.
		this.startedAtMs = performance.now();
		this.audioGraph?.setMasterVolume(edit.masterVolumeDb);
		const now = ctx.currentTime;
		let lastEndMs = 0;
		for (const {
			segment,
			buffer,
			rendered,
			overlapStart,
			overlapEnd,
		} of prepared) {
			const preprocessed = rendered !== null;
			const node = ctx.createBufferSource();
			node.buffer = rendered ?? buffer;
			node.detune.value = 0;
			node.playbackRate.value = preprocessed
				? 1
				: segment.effects.reverse
					? -segment.effects.rate
					: segment.effects.rate;
			const graph = this.audioGraph?.createSegment(
				segment.effects,
				preprocessed,
			);
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
				preprocessed
					? overlapStart - segment.timelineStart
					: sourceWindow.offset,
				preprocessed ? overlapEnd - overlapStart : sourceWindow.duration,
			);
			this.active.set(segment.id, { node, graph, preprocessed });
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
		this.playbackGeneration += 1;
		this.pendingPlayback = false;
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
		if (active.preprocessed) return;
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
		if (this.ctx?.state !== "closed") void this.ctx?.close();
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
