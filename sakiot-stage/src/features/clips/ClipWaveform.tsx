import { useEffect, useMemo, useState } from "react";
import { useGetClipWaveformQuery } from "../../app/apiSlice";
import { Button, ProgressBar } from "../../shared/ui";
import { WaveformCanvas } from "../audio-dashboard/WaveformCanvas";
import {
	decodeWaveformPeaks,
	EMPTY_WAVEFORM_ENVELOPE,
} from "../audio-dashboard/waveformPeaks";

const CLIP_WAVEFORM_HEIGHT_PX = 140;

export function ClipWaveform(props: {
	guildId: string;
	clipId: string;
	positionSeconds: number;
	durationSeconds: number;
	onSeek: (seconds: number) => void;
}) {
	const [requestKey, setRequestKey] = useState<number | undefined>();
	const [generating, setGenerating] = useState(true);
	const { currentData: data, isError } = useGetClipWaveformQuery(
		{
			guild_id: props.guildId,
			clip_id: props.clipId,
			timestamp: requestKey,
		},
		{
			pollingInterval: generating ? 1_000 : 0,
		},
	);

	useEffect(() => {
		if (data?.progress === 100 || data?.error || isError) setGenerating(false);
	}, [data?.error, data?.progress, isError]);

	const peaks = useMemo(
		() =>
			data?.data ? decodeWaveformPeaks(data.data) : EMPTY_WAVEFORM_ENVELOPE,
		[data?.data],
	);

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
		<div className="relative my-4 h-35 [border-radius:1px] overflow-hidden [background-color:rgba(168,_85,_247,_0.18)]">
			{generating && !waveformError && (
				<div className="absolute top-0 left-0 right-0 [z-index:2] px-2 py-1 [background-color:rgba(15,_23,_42,_0.78)] pointer-events-none">
					<span className="text-xs leading-5">
						Building clip waveform ({progress}%)
					</span>
					<ProgressBar value={progress} />
				</div>
			)}
			{!data?.data && !generating && !waveformError && (
				<div className="absolute inset-0 grid [place-items:center] pointer-events-none">
					<span className="text-muted text-xs leading-5">
						Clip waveform has not been built.
					</span>
				</div>
			)}
			{waveformError && (
				<div className="absolute inset-0 grid [place-items:center]">
					<span className="text-danger text-xs leading-5">
						Clip waveform unavailable.
					</span>
				</div>
			)}
			<WaveformCanvas
				peaks={peaks}
				height={CLIP_WAVEFORM_HEIGHT_PX}
				label="Clip waveform"
				onSeekFraction={
					data?.data
						? (fraction) => props.onSeek(fraction * props.durationSeconds)
						: undefined
				}
			/>
			{!data?.data && (
				<Button
					className="absolute right-2 bottom-2 [z-index:3]"
					variant="primary"
					size="sm"
					isDisabled={generating}
					onPress={() => {
						setGenerating(true);
						setRequestKey(Date.now());
					}}
				>
					{waveformError ? "Retry waveform" : "Build waveform"}
				</Button>
			)}
			<div
				className="absolute top-0 bottom-0 w-0.5 [background-color:white] pointer-events-none"
				style={{ left: `${playhead}%` }}
			/>
		</div>
	);
}
