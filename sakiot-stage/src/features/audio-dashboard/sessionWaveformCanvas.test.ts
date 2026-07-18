import { describe, expect, it, mock } from "bun:test";
import {
	drawSessionWaveform,
	type WaveformCanvasContext,
} from "./sessionWaveformCanvas";

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

describe("drawSessionWaveform", () => {
	it("clears stale pixels when current recording has no waveform", () => {
		const context = canvasContext();

		drawSessionWaveform(context, 320, 140, []);

		expect(context.clearRect).toHaveBeenCalledWith(0, 0, 320, 140);
		expect(context.stroke).not.toHaveBeenCalled();
	});

	it("clears before drawing current waveform", () => {
		const context = canvasContext();

		drawSessionWaveform(context, 2, 140, [-1, 1]);

		expect(context.clearRect).toHaveBeenCalledWith(0, 0, 2, 140);
		expect(context.stroke).toHaveBeenCalledTimes(1);
	});
});
