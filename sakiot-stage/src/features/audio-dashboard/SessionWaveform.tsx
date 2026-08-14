import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import LinearProgress from "@mui/material/LinearProgress";
import Typography from "@mui/material/Typography";
import type { ReactNode } from "react";
import { useCallback, useEffect, useState } from "react";
import {
	useRebuildSessionWaveformMutation,
	useRebuildSilenceFreeSessionWaveformMutation,
} from "../../app/apiSlice";
import { formatSessionTimecode } from "../../utils/formatTime";
import { TimelineGrid, TimelinePlayhead } from "./timelineLayout";
import { useSessionWaveformPeaks, WaveformCanvas } from "./WaveformCanvas";
import type { WaveformEnvelope } from "./waveformPeaks";

const WAVEFORM_HEIGHT_PX = 132;

export function SessionWaveform(props: {
	sessionId: string;
	positionMs: number;
	durationMs: number;
	onSeek: (positionMs: number) => void;
	silenceFree?: boolean;
}) {
	const [rebuilding, setRebuilding] = useState(false);
	const [rebuildProgress, setRebuildProgress] = useState(0);
	const { query, peaks } = useSessionWaveformPeaks(
		props.sessionId,
		props.silenceFree,
	);
	const { currentData: data, isError, refetch } = query;
	const [rebuildNormalWaveform, normalRebuildState] =
		useRebuildSessionWaveformMutation();
	const [rebuildSilenceFreeWaveform, silenceFreeRebuildState] =
		useRebuildSilenceFreeSessionWaveformMutation();
	const rebuildState = props.silenceFree
		? silenceFreeRebuildState
		: normalRebuildState;
	const waveformName = props.silenceFree ? "Silence-free" : "Logical session";

	const pollRebuild = useCallback(async () => {
		const result = await refetch();
		if (result.data) setRebuildProgress(result.data.progress);
		if (result.data?.building === false) {
			setRebuilding(false);
			return false;
		}
		if (result.error) {
			// Transient fetch error — keep polling instead of abandoning the rebuild.
			return true;
		}
		return true;
	}, [refetch]);

	useEffect(() => {
		if (data?.building) {
			setRebuildProgress(data.progress);
			setRebuilding(true);
		}
	}, [data?.building, data?.progress]);

	useEffect(() => {
		if (!rebuilding) return;
		let cancelled = false;
		const tick = async () => {
			const shouldContinue = await pollRebuild();
			if (cancelled || !shouldContinue) {
				window.clearInterval(interval);
				return;
			}
		};
		const interval = window.setInterval(() => void tick(), 1_000);
		void tick();
		return () => {
			cancelled = true;
			window.clearInterval(interval);
		};
	}, [pollRebuild, rebuilding]);

	const startRebuild = async () => {
		setRebuildProgress(0);
		try {
			const rebuild = props.silenceFree
				? rebuildSilenceFreeWaveform
				: rebuildNormalWaveform;
			await rebuild(props.sessionId).unwrap();
			setRebuilding(true);
		} catch {
			setRebuilding(false);
		}
	};

	const buildInProgress =
		rebuilding || rebuildState.isLoading || data?.building === true;
	const waveformError = isError || rebuildState.isError;

	return (
		<SessionWaveformDisplay
			peaks={peaks}
			positionMs={props.positionMs}
			durationMs={props.durationMs}
			onSeek={props.onSeek}
			label={`${waveformName} logical recording waveform`}
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
						Building {waveformName.toLowerCase()} waveform ({rebuildProgress}%)
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
						{waveformName} waveform has not been built.
						{!props.silenceFree &&
							" Channel Mix uses separate physical-source waveforms."}
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
						{waveformName} waveform unavailable.
					</Typography>
				</Box>
			)}
			<Button
				size="small"
				variant="contained"
				onClick={() => void startRebuild()}
				disabled={buildInProgress}
				sx={{ position: "absolute", right: 8, bottom: 8, zIndex: 4 }}
			>
				{data?.data ? "Rebuild waveform" : "Build waveform"}
			</Button>
		</SessionWaveformDisplay>
	);
}

export function SessionWaveformDisplay(props: {
	peaks: WaveformEnvelope;
	positionMs: number;
	durationMs: number;
	onSeek: (positionMs: number) => void;
	label: string;
	children?: ReactNode;
}) {
	const [hoverFraction, setHoverFraction] = useState<number | null>(null);
	const playhead =
		props.durationMs > 0
			? Math.min(100, Math.max(0, (props.positionMs / props.durationMs) * 100))
			: 0;

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
			<WaveformCanvas
				peaks={props.peaks}
				height={WAVEFORM_HEIGHT_PX}
				label={props.label}
				onSeekFraction={(fraction) => props.onSeek(fraction * props.durationMs)}
				onHoverFraction={setHoverFraction}
			/>
			<TimelineGrid />
			{hoverFraction !== null && (
				<Box
					aria-hidden="true"
					sx={{
						position: "absolute",
						top: 0,
						bottom: 0,
						left: `${hoverFraction * 100}%`,
						borderLeft: "1px solid rgba(125, 211, 252, 0.85)",
						pointerEvents: "none",
						zIndex: 3,
					}}
				>
					<Typography
						variant="caption"
						sx={{
							position: "absolute",
							top: 6,
							px: 0.75,
							py: 0.25,
							borderRadius: 0.75,
							bgcolor: "rgba(2, 6, 23, 0.9)",
							color: "info.light",
							fontVariantNumeric: "tabular-nums",
							whiteSpace: "nowrap",
							transform:
								hoverFraction < 0.1
									? "translateX(4px)"
									: hoverFraction > 0.9
										? "translateX(calc(-100% - 4px))"
										: "translateX(-50%)",
						}}
					>
						{formatSessionTimecode(
							(hoverFraction * props.durationMs) / 1_000,
							props.durationMs / 1_000,
						)}
					</Typography>
				</Box>
			)}
			{props.children}
			<TimelinePlayhead percent={playhead} />
		</Box>
	);
}
