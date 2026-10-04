import { useEffect, useRef } from "react";
import {
	useGetSessionWaveformQuery,
	useGetSilenceFreeSessionWaveformQuery,
} from "../../app/apiSlice";
import { fractionInTarget } from "../../shared/geometry";
import { drawSessionWaveform } from "./sessionWaveformCanvas";
import { useDecodedPeaks, type WaveformEnvelope } from "./waveformPeaks";

/**
 * Decoded peaks for a session. The query is cached by RTK, so the overview and
 * the clip editor share one request; only the decode is per caller.
 */
export function useSessionWaveformPeaks(
	sessionId: string,
	silenceFree = false,
) {
	const normalQuery = useGetSessionWaveformQuery(sessionId, {
		skip: silenceFree,
	});
	const silenceFreeQuery = useGetSilenceFreeSessionWaveformQuery(sessionId, {
		skip: !silenceFree,
	});
	const query = silenceFree ? silenceFreeQuery : normalQuery;
	const encoded = query.currentData?.data;
	const peaks = useDecodedPeaks(encoded);
	return { query, peaks };
}

export function WaveformCanvas(props: {
	peaks: WaveformEnvelope;
	height: number;
	label: string;
	/** Portion of the recording to draw; defaults to all of it. */
	startFraction?: number;
	endFraction?: number;
	/**
	 * Length of the recording, when known. Peaks are then placed by time, so a
	 * waveform built partway through a live recording leaves the rest blank
	 * instead of stretching out of step with playback.
	 */
	durationMs?: number;
	onSeekFraction?: (fraction: number) => void;
	onHoverFraction?: (fraction: number | null) => void;
}) {
	const canvasRef = useRef<HTMLCanvasElement | null>(null);
	const {
		peaks,
		height,
		startFraction = 0,
		endFraction = 1,
		durationMs,
	} = props;

	useEffect(() => {
		const canvas = canvasRef.current;
		if (!canvas) return;

		const draw = () => {
			const ratio = window.devicePixelRatio || 1;
			const width = canvas.clientWidth;
			canvas.width = Math.max(1, Math.floor(width * ratio));
			canvas.height = Math.max(1, Math.floor(height * ratio));
			const context = canvas.getContext("2d");
			if (!context) return;
			context.scale(ratio, ratio);
			drawSessionWaveform(context, width, height, peaks, {
				startFraction,
				endFraction,
				durationMs,
			});
		};

		draw();
		const observer = new ResizeObserver(draw);
		observer.observe(canvas);
		return () => observer.disconnect();
	}, [durationMs, endFraction, height, peaks, startFraction]);

	return (
		<canvas
			ref={canvasRef}
			aria-label={props.label}
			onClick={
				props.onSeekFraction &&
				((event) => props.onSeekFraction?.(fractionInTarget(event)))
			}
			onPointerMove={
				props.onHoverFraction &&
				((event) => props.onHoverFraction?.(fractionInTarget(event)))
			}
			onPointerLeave={
				props.onHoverFraction && (() => props.onHoverFraction?.(null))
			}
			style={{
				width: "100%",
				height,
				display: "block",
				cursor: props.onSeekFraction ? "pointer" : "inherit",
			}}
		/>
	);
}
