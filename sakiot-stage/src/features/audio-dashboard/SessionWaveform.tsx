import Box from "@mui/material/Box";
import LinearProgress from "@mui/material/LinearProgress";
import Typography from "@mui/material/Typography";
import { useEffect, useRef } from "react";
import { useGetSessionWaveformQuery } from "../../app/apiSlice";

function decodePeaks(base64: string): number[] {
	const binary = atob(base64);
	const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
	if (bytes.byteLength < 20) return [];
	const view = new DataView(bytes.buffer);
	const flags = view.getUint32(4, true);
	const length = view.getUint32(16, true);
	const sixteenBit = flags === 1;
	const peaks: number[] = [];
	let offset = 20;
	for (let index = 0; index < length; index += 1) {
		if (sixteenBit) {
			if (offset + 2 > view.byteLength) break;
			peaks.push(view.getInt16(offset, true) / 32768);
			offset += 2;
		} else {
			if (offset + 1 > view.byteLength) break;
			peaks.push(view.getInt8(offset) / 128);
			offset += 1;
		}
	}
	return peaks;
}

export function SessionWaveform(props: {
	sessionId: string;
	positionMs: number;
	durationMs: number;
	onSeek: (positionMs: number) => void;
}) {
	const canvasRef = useRef<HTMLCanvasElement | null>(null);
	const { data, isError } = useGetSessionWaveformQuery(props.sessionId, {
		pollingInterval: 1_000,
	});

	useEffect(() => {
		const canvas = canvasRef.current;
		if (!canvas || !data?.data) return;
		const peaks = decodePeaks(data.data);
		if (peaks.length === 0) return;

		const draw = () => {
			const ratio = window.devicePixelRatio || 1;
			const width = canvas.clientWidth;
			const height = canvas.clientHeight;
			canvas.width = Math.max(1, Math.floor(width * ratio));
			canvas.height = Math.max(1, Math.floor(height * ratio));
			const context = canvas.getContext("2d");
			if (!context) return;
			context.scale(ratio, ratio);
			context.clearRect(0, 0, width, height);
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
		};

		draw();
		const observer = new ResizeObserver(draw);
		observer.observe(canvas);
		return () => observer.disconnect();
	}, [data?.data]);

	const playhead =
		props.durationMs > 0
			? Math.min(100, Math.max(0, (props.positionMs / props.durationMs) * 100))
			: 0;

	return (
		<Box sx={{ position: "relative", my: 2 }}>
			{data?.progress !== 100 && !isError && (
				<Box sx={{ mb: 1 }}>
					<Typography variant="caption">
						Building combined waveform ({data?.progress ?? 0}%)
					</Typography>
					<LinearProgress variant="determinate" value={data?.progress ?? 0} />
				</Box>
			)}
			{isError && (
				<Typography color="error" variant="caption">
					Combined waveform unavailable.
				</Typography>
			)}
			<canvas
				ref={canvasRef}
				aria-label="Combined logical recording waveform"
				onClick={(event) => {
					const bounds = event.currentTarget.getBoundingClientRect();
					const fraction =
						(event.clientX - bounds.left) / Math.max(1, bounds.width);
					props.onSeek(fraction * props.durationMs);
				}}
				style={{
					width: "100%",
					height: 140,
					display: "block",
					cursor: "pointer",
					borderRadius: 6,
				}}
			/>
			<Box
				sx={{
					position: "absolute",
					top: data?.progress !== 100 && !isError ? 44 : 0,
					bottom: 0,
					left: `${playhead}%`,
					width: 2,
					bgcolor: "white",
					pointerEvents: "none",
				}}
			/>
		</Box>
	);
}
