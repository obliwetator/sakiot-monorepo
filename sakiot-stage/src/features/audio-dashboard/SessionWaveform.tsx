import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import LinearProgress from "@mui/material/LinearProgress";
import Typography from "@mui/material/Typography";
import { useCallback, useEffect, useRef, useState } from "react";
import {
	useGetSessionWaveformQuery,
	useRebuildSessionWaveformMutation,
} from "../../app/apiSlice";

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
	const [rebuilding, setRebuilding] = useState(false);
	const [rebuildProgress, setRebuildProgress] = useState(0);
	const { data, isError, refetch } = useGetSessionWaveformQuery(
		props.sessionId,
	);
	const [rebuildWaveform, rebuildState] = useRebuildSessionWaveformMutation();

	const pollRebuild = useCallback(async () => {
		const result = await refetch();
		if (result.data) setRebuildProgress(result.data.progress);
		if (result.error || result.data?.building === false) {
			setRebuilding(false);
		}
	}, [refetch]);

	useEffect(() => {
		if (data?.building) {
			setRebuildProgress(data.progress);
			setRebuilding(true);
		}
	}, [data?.building, data?.progress]);

	useEffect(() => {
		if (!rebuilding) return;
		void pollRebuild();
		const interval = window.setInterval(() => void pollRebuild(), 1_000);
		return () => window.clearInterval(interval);
	}, [pollRebuild, rebuilding]);

	const startRebuild = async () => {
		setRebuildProgress(0);
		try {
			await rebuildWaveform(props.sessionId).unwrap();
			setRebuilding(true);
		} catch {
			setRebuilding(false);
		}
	};

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
	const buildInProgress =
		rebuilding || rebuildState.isLoading || data?.building === true;
	const waveformError = isError || rebuildState.isError;

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
			{buildInProgress && !waveformError && (
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
						Building combined waveform ({rebuildProgress}%)
					</Typography>
					<LinearProgress variant="determinate" value={rebuildProgress} />
				</Box>
			)}
			{!data?.data && !buildInProgress && !waveformError && (
				<Box
					sx={{
						position: "absolute",
						inset: 0,
						display: "grid",
						placeItems: "center",
						zIndex: 1,
						pointerEvents: "none",
					}}
				>
					<Typography color="text.secondary" variant="caption">
						Combined waveform has not been built.
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
						zIndex: 2,
						pointerEvents: "none",
					}}
				>
					<Typography color="error" variant="caption">
						Combined waveform unavailable.
					</Typography>
				</Box>
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
			<Button
				size="small"
				variant="contained"
				onClick={() => void startRebuild()}
				disabled={buildInProgress}
				sx={{ position: "absolute", right: 8, bottom: 8, zIndex: 3 }}
			>
				{data?.data ? "Rebuild waveform" : "Build waveform"}
			</Button>
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
