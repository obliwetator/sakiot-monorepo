import { describe, expect, it, mock } from "bun:test";
import {
	drawSessionWaveform,
	type WaveformCanvasContext,
} from "./sessionWaveformCanvas";
import { EMPTY_WAVEFORM_ENVELOPE } from "./waveformPeaks";

function canvasContext(): WaveformCanvasContext {
	return {
		fillStyle: "",
		strokeStyle: "",
		lineWidth: 0,
		clearRect: mock(() => {}),
		fillRect: mock(() => {}),
		beginPath: mock(() => {}),
		moveTo: mock(() => {}),
		lineTo: mock(() => {}),
		stroke: mock(() => {}),
	};
}

/** Vertical extents drawn per column, as [top, bottom] canvas coordinates. */
function drawnColumns(context: WaveformCanvasContext): [number, number][] {
	const tops = (context.moveTo as ReturnType<typeof mock>).mock.calls;
	const bottoms = (context.lineTo as ReturnType<typeof mock>).mock.calls;
	return tops.map((top, index) => [top[1], bottoms[index][1]]);
}

describe("drawSessionWaveform", () => {
	it("clears stale pixels when current recording has no waveform", () => {
		const context = canvasContext();

		drawSessionWaveform(context, 320, 140, EMPTY_WAVEFORM_ENVELOPE);

		expect(context.clearRect).toHaveBeenCalledWith(0, 0, 320, 140);
		expect(context.stroke).not.toHaveBeenCalled();
	});

	it("clears before drawing current waveform", () => {
		const context = canvasContext();

		drawSessionWaveform(context, 2, 140, { min: [-1], max: [1] });

		expect(context.clearRect).toHaveBeenCalledWith(0, 0, 2, 140);
		expect(context.stroke).toHaveBeenCalledTimes(1);
	});

	it("draws each point around the centre line", () => {
		const context = canvasContext();

		drawSessionWaveform(context, 2, 100, { min: [-1, -0.5], max: [1, 0.5] });

		expect(drawnColumns(context)).toEqual([
			[0, 100],
			[25, 75],
		]);
	});

	it("keeps the loudest sample when a column covers many points", () => {
		const context = canvasContext();

		drawSessionWaveform(context, 1, 100, {
			min: [-0.1, -1, -0.1],
			max: [0.1, 0.2, 0.1],
		});

		expect(drawnColumns(context)).toEqual([[40, 100]]);
	});

	it("draws only the windowed slice of the recording", () => {
		const context = canvasContext();

		drawSessionWaveform(
			context,
			1,
			100,
			{ min: [-1, -0.5, -0.25, 0], max: [1, 0.5, 0.25, 0] },
			{ startFraction: 0.5, endFraction: 0.75 },
		);

		expect(drawnColumns(context)).toEqual([[37.5, 62.5]]);
	});

	it("survives a window given back to front", () => {
		const context = canvasContext();

		drawSessionWaveform(
			context,
			2,
			100,
			{ min: [-1, -1], max: [1, 1] },
			{ startFraction: 1, endFraction: 0 },
		);

		expect(context.stroke).toHaveBeenCalledTimes(1);
	});

	it("mirrors the window horizontally when reversed", () => {
		const context = canvasContext();

		drawSessionWaveform(
			context,
			2,
			100,
			{ min: [-1, -0.5], max: [1, 0.5] },
			undefined,
			{ reverse: true },
		);

		// Column 0 draws the last point and column 1 the first: the forward
		// order [0, 100], [25, 75] is drawn in reverse.
		expect(drawnColumns(context)).toEqual([
			[25, 75],
			[0, 100],
		]);
	});

	it("keeps the loudest sample of a column when reversed", () => {
		const context = canvasContext();

		drawSessionWaveform(
			context,
			1,
			100,
			{ min: [-0.1, -1, -0.1], max: [0.1, 0.2, 0.1] },
			undefined,
			{ reverse: true },
		);

		// The whole point range still contributes to the column envelope,
		// so the middle sample (the loudest) is kept.
		expect(drawnColumns(context)).toEqual([[40, 100]]);
	});

	it("never samples past the point range when reversed", () => {
		const context = canvasContext();

		drawSessionWaveform(
			context,
			3,
			100,
			{ min: [-1, -0.5, 0], max: [1, 0.5, 0] },
			undefined,
			{ reverse: true },
		);

		// Column 0 draws the silent last point as a flat centre line, column 1
		// the middle sample, column 2 the loudest point - all inside bounds.
		expect(drawnColumns(context)).toEqual([
			[50, 50],
			[25, 75],
			[0, 100],
		]);
	});
});
