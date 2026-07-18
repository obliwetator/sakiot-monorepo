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

export function drawSessionWaveform(
	context: WaveformCanvasContext,
	width: number,
	height: number,
	peaks: readonly number[],
) {
	context.clearRect(0, 0, width, height);
	if (peaks.length === 0) return;

	context.fillStyle = "rgba(168, 85, 247, 0.18)";
	context.fillRect(0, 0, width, height);
	const center = height / 2;
	context.strokeStyle = "#d946ef";
	context.lineWidth = 1;
	context.beginPath();
	for (let x = 0; x < width; x += 1) {
		const index = Math.min(
			peaks.length - 1,
			Math.floor((x / Math.max(1, width - 1)) * peaks.length),
		);
		const amplitude = Math.abs(peaks[index] ?? 0) * center;
		context.moveTo(x, center - amplitude);
		context.lineTo(x, center + amplitude);
	}
	context.stroke();
}
