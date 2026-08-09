import type { WaveformEnvelope } from "./waveformPeaks";

export interface WaveformCanvasContext {
	fillStyle: string | CanvasGradient | CanvasPattern;
	strokeStyle: string | CanvasGradient | CanvasPattern;
	lineWidth: number;
	clearRect: (x: number, y: number, width: number, height: number) => void;
	fillRect: (x: number, y: number, width: number, height: number) => void;
	beginPath: () => void;
	moveTo: (x: number, y: number) => void;
	lineTo: (x: number, y: number) => void;
	stroke: () => void;
}

/** Fractions of the recording to draw; defaults to all of it. */
export interface WaveformWindow {
	startFraction: number;
	endFraction: number;
}

/** Color overrides for the waveform bars and its backdrop fill. */
export interface WaveformStyle {
	/** Bar stroke color. */
	strokeStyle?: string;
	/**
	 * Backdrop fill, or null to skip the fill entirely (e.g. when drawing on
	 * top of an already tinted surface such as a timeline segment).
	 */
	fillStyle?: string | null;
	/**
	 * Mirrors the window horizontally: the right edge of the source window is
	 * drawn at the left of the canvas, so a reversed segment shows its audio
	 * playing backwards.
	 */
	reverse?: boolean;
}

const FULL_WINDOW: WaveformWindow = { startFraction: 0, endFraction: 1 };

export function drawSessionWaveform(
	context: WaveformCanvasContext,
	width: number,
	height: number,
	peaks: WaveformEnvelope,
	window: WaveformWindow = FULL_WINDOW,
	style: WaveformStyle = {},
) {
	context.clearRect(0, 0, width, height);
	const pointCount = Math.min(peaks.min.length, peaks.max.length);
	if (pointCount === 0 || width <= 0) return;

	if (style.fillStyle !== null) {
		context.fillStyle = style.fillStyle ?? "rgba(168, 85, 247, 0.18)";
		context.fillRect(0, 0, width, height);
	}
	const center = height / 2;
	context.strokeStyle = style.strokeStyle ?? "#d946ef";
	context.lineWidth = 1;
	context.beginPath();

	const from = clampFraction(window.startFraction) * pointCount;
	const to = clampFraction(window.endFraction) * pointCount;
	// Reversed segments walk the source window from its end, so column 0
	// samples the point the playback will reach last.
	const start = style.reverse ? to : from;
	const end = style.reverse ? from : to;

	for (let x = 0; x < width; x += 1) {
		// Every point falling in this column contributes, so raising the peak
		// resolution sharpens the envelope instead of aliasing it into noise.
		// Reversed windows walk the span downwards; the raw endpoints still
		// delimit the same column, so aggregate their range either way.
		const rawStart = start + (x / width) * (end - start);
		const rawEnd = start + ((x + 1) / width) * (end - start);
		const first = Math.floor(Math.min(rawStart, rawEnd));
		const last = Math.max(first + 1, Math.ceil(Math.max(rawStart, rawEnd)));
		let min = 0;
		let max = 0;
		for (
			let point = Math.max(0, first);
			point < Math.min(pointCount, last);
			point += 1
		) {
			min = Math.min(min, peaks.min[point]);
			max = Math.max(max, peaks.max[point]);
		}
		context.moveTo(x, center - clampAmplitude(max) * center);
		context.lineTo(x, center - clampAmplitude(min) * center);
	}
	context.stroke();
}

function clampFraction(value: number): number {
	if (!Number.isFinite(value)) return 0;
	return Math.min(1, Math.max(0, value));
}

function clampAmplitude(value: number): number {
	if (!Number.isFinite(value)) return 0;
	return Math.min(1, Math.max(-1, value));
}
