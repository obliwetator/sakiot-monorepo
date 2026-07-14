import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import LinearProgress from "@mui/material/LinearProgress";
import Typography from "@mui/material/Typography";
import { useEffect, useRef, useState } from "react";
import { useGetClipWaveformQuery } from "../../app/apiSlice";
import { decodeWaveformPeaks } from "../audio-dashboard/waveformPeaks";

export function ClipWaveform(props: {
	guildId: string;
	clipId: string;
	positionSeconds: number;
	durationSeconds: number;
	onSeek: (seconds: number) => void;
}) {
	const canvasRef = useRef<HTMLCanvasElement | null>(null);
	const [requestKey, setRequestKey] = useState<number | null>(null);
	const [generating, setGenerating] = useState(false);
	const { data, isError } = useGetClipWaveformQuery(
		{
			guild_id: props.guildId,
			clip_id: props.clipId,
			timestamp: requestKey ?? undefined,
		},
		{
			skip: requestKey === null,
			pollingInterval: generating ? 1_000 : 0,
		},
	);

	useEffect(() => {
		if (data?.progress === 100 || data?.error || isError) setGenerating(false);
	}, [data?.error, data?.progress, isError]);

	useEffect(() => {
		const canvas = canvasRef.current;
		if (!canvas || !data?.data) return;
		const peaks = decodeWaveformPeaks(data.data);
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

	const progress = data?.progress ?? 0;
	const waveformError = isError || Boolean(data?.error);
	const playhead =
		props.durationSeconds > 0
			? Math.min(
					100,
					Math.max(0, (props.positionSeconds / props.durationSeconds) * 100),
				)
			: 0;

	return (
		<Box
			sx={{
				position: "relative",
				my: 2,
				height: 140,
				borderRadius: 1,
				overflow: "hidden",
				bgcolor: "rgba(168, 85, 247, 0.18)",
			}}
		>
			{generating && !waveformError && (
				<Box
					sx={{
						position: "absolute",
						top: 0,
						left: 0,
						right: 0,
						zIndex: 2,
						px: 1,
						py: 0.5,
						bgcolor: "rgba(15, 23, 42, 0.78)",
						pointerEvents: "none",
					}}
				>
					<Typography variant="caption">
						Building clip waveform ({progress}%)
					</Typography>
					<LinearProgress variant="determinate" value={progress} />
				</Box>
			)}
			{!data?.data && !generating && !waveformError && (
				<Box
					sx={{
						position: "absolute",
						inset: 0,
						display: "grid",
						placeItems: "center",
						pointerEvents: "none",
					}}
				>
					<Typography color="text.secondary" variant="caption">
						Clip waveform has not been loaded.
					</Typography>
				</Box>
			)}
			{waveformError && (
				<Box
					sx={{
						position: "absolute",
						inset: 0,
						display: "grid",
						placeItems: "center",
					}}
				>
					<Typography color="error" variant="caption">
						Clip waveform unavailable.
					</Typography>
				</Box>
			)}
			<canvas
				ref={canvasRef}
				aria-label="Clip waveform"
				onClick={(event) => {
					if (!data?.data) return;
					const bounds = event.currentTarget.getBoundingClientRect();
					const fraction =
						(event.clientX - bounds.left) / Math.max(1, bounds.width);
					props.onSeek(fraction * props.durationSeconds);
				}}
				style={{
					width: "100%",
					height: 140,
					display: "block",
					cursor: data?.data ? "pointer" : "default",
				}}
			/>
			{!data?.data && (
				<Button
					size="small"
					variant="contained"
					onClick={() => {
						setGenerating(true);
						setRequestKey(Date.now());
					}}
					disabled={generating}
					sx={{ position: "absolute", right: 8, bottom: 8, zIndex: 3 }}
				>
					Build waveform
				</Button>
			)}
			<Box
				sx={{
					position: "absolute",
					top: 0,
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
