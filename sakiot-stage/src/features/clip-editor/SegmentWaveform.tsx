import { useEffect, useMemo, useRef } from "react";
import { drawSessionWaveform } from "../audio-dashboard/sessionWaveformCanvas";
import type { WaveformEnvelope } from "../audio-dashboard/waveformPeaks";

/**
 * The portion of the source clip's waveform a segment covers, as fractions
 * of the whole clip. Null when the range or duration is unusable.
 */
export function waveformWindowFractions(
	sourceIn: number,
	sourceOut: number,
	durationSec: number,
): { startFraction: number; endFraction: number } | null {
	if (!Number.isFinite(durationSec) || durationSec <= 0) return null;
	const startFraction = Math.min(1, Math.max(0, sourceIn / durationSec));
	const endFraction = Math.min(1, Math.max(0, sourceOut / durationSec));
	return endFraction > startFraction ? { startFraction, endFraction } : null;
}

/**
 * Draws the source clip's waveform inside a timeline segment box, cropped to
 * the segment's [sourceIn, sourceOut] window. The box width already encodes
 * the rate effect, so the peaks stretch to fill it at any playback speed.
 * Reversed segments mirror the window so the drawing matches the backwards
 * playback. Renders nothing while peaks are unavailable.
 */
export function SegmentWaveform(props: {
	peaks: WaveformEnvelope;
	sourceIn: number;
	sourceOut: number;
	durationSec: number;
	selected: boolean;
	reverse: boolean;
}) {
	const canvasRef = useRef<HTMLCanvasElement | null>(null);
	const { peaks, selected } = props;
	const fractions = useMemo(
		() =>
			waveformWindowFractions(
				props.sourceIn,
				props.sourceOut,
				props.durationSec,
			),
		[props.durationSec, props.sourceIn, props.sourceOut],
	);

	useEffect(() => {
		const canvas = canvasRef.current;
		if (!canvas || !fractions) return;
		const draw = () => {
			const ratio = window.devicePixelRatio || 1;
			const width = canvas.clientWidth;
			const height = canvas.clientHeight;
			if (width <= 0 || height <= 0) return;
			canvas.width = Math.max(1, Math.floor(width * ratio));
			canvas.height = Math.max(1, Math.floor(height * ratio));
			const context = canvas.getContext("2d");
			if (!context) return;
			context.scale(ratio, ratio);
			drawSessionWaveform(context, width, height, peaks, fractions, {
				fillStyle: null,
				strokeStyle: selected
					? "rgba(255, 255, 255, 0.9)"
					: "rgba(255, 255, 255, 0.45)",
				reverse: props.reverse,
			});
		};
		draw();
		const observer = new ResizeObserver(draw);
		observer.observe(canvas);
		return () => observer.disconnect();
	}, [fractions, peaks, props.reverse, selected]);

	if (!fractions) return null;
	return (
		<canvas
			ref={canvasRef}
			style={{
				position: "absolute",
				inset: 0,
				width: "100%",
				height: "100%",
				pointerEvents: "none",
			}}
		/>
	);
}
