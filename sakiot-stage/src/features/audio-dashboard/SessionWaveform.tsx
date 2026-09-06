import type { ReactNode } from "react";
import { useCallback, useEffect, useState } from "react";
import {
	useRebuildSessionWaveformMutation,
	useRebuildSilenceFreeSessionWaveformMutation,
} from "../../app/apiSlice";
import { Button, ProgressBar } from "../../shared/ui";
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
				<div className="absolute top-0 left-0 right-0 [z-index:2] px-2 py-1 [background-color:rgba(15,_23,_42,_0.78)] pointer-events-none">
					<span className="text-xs leading-5">
						Building {waveformName.toLowerCase()} waveform ({rebuildProgress}%)
					</span>
					<ProgressBar value={rebuildProgress} />
				</div>
			)}
			{!data?.data && !buildInProgress && !waveformError && (
				<div className="absolute inset-0 grid [place-items:center] [z-index:1] pointer-events-none">
					<span className="text-muted text-xs leading-5">
						{waveformName} waveform has not been built.
						{!props.silenceFree &&
							" Channel Mix uses separate physical-source waveforms."}
					</span>
				</div>
			)}
			{waveformError && (
				<div className="absolute inset-0 grid [place-items:center] [z-index:2] pointer-events-none">
					<span className="text-danger text-xs leading-5">
						{waveformName} waveform unavailable.
					</span>
				</div>
			)}
			<Button
				className="absolute right-2 bottom-2 [z-index:4]"
				variant="primary"
				size="sm"
				isDisabled={buildInProgress}
				onPress={() => void startRebuild()}
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
		<div
			className="relative [border-radius:1px] overflow-hidden [background-color:rgba(168,_85,_247,_0.18)]"
			style={{ height: WAVEFORM_HEIGHT_PX }}
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
				<div
					aria-hidden="true"
					className="absolute top-0 bottom-0 [border-left:1px_solid_rgba(125,_211,_252,_0.85)] pointer-events-none [z-index:3]"
					style={{ left: `${hoverFraction * 100}%` }}
				>
					<span
						className="text-xs leading-5 absolute top-1.5 px-1.5 py-0.5 [border-radius:0.75px] [background-color:rgba(2,_6,_23,_0.9)] [color:#7dd3fc] tabular-nums whitespace-nowrap"
						style={{
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
					</span>
				</div>
			)}
			{props.children}
			<TimelinePlayhead percent={playhead} />
		</div>
	);
}
