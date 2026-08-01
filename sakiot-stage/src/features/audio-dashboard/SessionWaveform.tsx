import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import LinearProgress from "@mui/material/LinearProgress";
import Typography from "@mui/material/Typography";
import { useCallback, useEffect, useState } from "react";
import { useRebuildSessionWaveformMutation } from "../../app/apiSlice";
import { TimelineGrid, TimelinePlayhead } from "./timelineLayout";
import { useSessionWaveformPeaks, WaveformCanvas } from "./WaveformCanvas";

const WAVEFORM_HEIGHT_PX = 132;

export function SessionWaveform(props: {
	sessionId: string;
	positionMs: number;
	durationMs: number;
	onSeek: (positionMs: number) => void;
}) {
	const [rebuilding, setRebuilding] = useState(false);
	const [rebuildProgress, setRebuildProgress] = useState(0);
	const { query, peaks } = useSessionWaveformPeaks(props.sessionId);
	const { currentData: data, isError, refetch } = query;
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
				height: WAVEFORM_HEIGHT_PX,
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
			<WaveformCanvas
				peaks={peaks}
				height={WAVEFORM_HEIGHT_PX}
				label="Combined logical recording waveform"
				onSeekFraction={(fraction) => props.onSeek(fraction * props.durationMs)}
			/>
			<TimelineGrid />
			<Button
				size="small"
				variant="contained"
				onClick={() => void startRebuild()}
				disabled={buildInProgress}
				sx={{ position: "absolute", right: 8, bottom: 8, zIndex: 4 }}
			>
				{data?.data ? "Rebuild waveform" : "Build waveform"}
			</Button>
			<TimelinePlayhead percent={playhead} />
		</Box>
	);
}
