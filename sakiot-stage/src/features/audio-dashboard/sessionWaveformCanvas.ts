import { alpha, palette } from "../../shared/palette";
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
	/**
	 * Length of the recording the fractions are of. With it, points are placed
	 * by their time instead of stretched to fill the window, so a waveform built
	 * partway through a live recording stops where its audio stops and the rest
	 * stays blank.
	 */
	durationMs?: number;
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

	// Points spanning the whole recording: more than exist when the waveform
	// covers only its start.
	const wholePoints =
		isPositive(window.durationMs) && isPositive(peaks.durationMs)
			? pointCount * (window.durationMs / peaks.durationMs)
			: pointCount;
	const from = clampFraction(window.startFraction) * wholePoints;
	const to = clampFraction(window.endFraction) * wholePoints;
	// Reversed segments walk the source window from its end, so column 0
	// samples the point the playback will reach last.
	const start = style.reverse ? to : from;
	const end = style.reverse ? from : to;

	const columns: { x: number; min: number; max: number }[] = [];
	for (let x = 0; x < width; x += 1) {
		// Every point falling in this column contributes, so raising the peak
		// resolution sharpens the envelope instead of aliasing it into noise.
		// Reversed windows walk the span downwards; the raw endpoints still
		// delimit the same column, so aggregate their range either way.
		const rawStart = start + (x / width) * (end - start);
		const rawEnd = start + ((x + 1) / width) * (end - start);
		const first = Math.max(0, Math.floor(Math.min(rawStart, rawEnd)));
		const last = Math.min(
			pointCount,
			Math.max(first + 1, Math.ceil(Math.max(rawStart, rawEnd))),
		);
		// Past the end of the waveform: left blank.
		if (first >= last) continue;
		let min = 0;
		let max = 0;
		for (let point = first; point < last; point += 1) {
			min = Math.min(min, peaks.min[point] ?? 0);
			max = Math.max(max, peaks.max[point] ?? 0);
		}
		columns.push({ x, min, max });
	}
	const firstColumn = columns[0];
	const lastColumn = columns.at(-1);
	if (!firstColumn || !lastColumn) return;

	if (style.fillStyle !== null) {
		context.fillStyle = style.fillStyle ?? alpha(palette.purple500, 0.18);
		context.fillRect(
			firstColumn.x,
			0,
			lastColumn.x + 1 - firstColumn.x,
			height,
		);
	}
	const center = height / 2;
	context.strokeStyle = style.strokeStyle ?? palette.fuchsia500;
	context.lineWidth = 1;
	context.beginPath();
	for (const { x, min, max } of columns) {
		context.moveTo(x, center - clampAmplitude(max) * center);
		context.lineTo(x, center - clampAmplitude(min) * center);
	}
	context.stroke();
}

function isPositive(value: number | undefined): value is number {
	return value !== undefined && Number.isFinite(value) && value > 0;
}

function clampFraction(value: number): number {
	if (!Number.isFinite(value)) return 0;
	return Math.min(1, Math.max(0, value));
}

function clampAmplitude(value: number): number {
	if (!Number.isFinite(value)) return 0;
	return Math.min(1, Math.max(-1, value));
}
