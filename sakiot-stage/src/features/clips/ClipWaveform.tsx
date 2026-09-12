import { useEffect, useMemo, useRef, useState } from "react";
import { useGetClipWaveformQuery } from "../../app/apiSlice";
import { Button, ProgressBar } from "../../shared/ui";
import { WaveformCanvas } from "../audio-dashboard/WaveformCanvas";
import {
	decodeWaveformPeaks,
	EMPTY_WAVEFORM_ENVELOPE,
} from "../audio-dashboard/waveformPeaks";
import {
	nextPollErrorCount,
	shouldKeepPollingClipWaveform,
} from "./clipWaveformPolling";

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
	const {
		currentData: data,
		isError,
		isFetching,
	} = useGetClipWaveformQuery(
		{
			guild_id: props.guildId,
			clip_id: props.clipId,
			timestamp: requestKey,
		},
		{
			pollingInterval: generating ? 1_000 : 0,
		},
	);

	// A poll can fail while the build is still in flight, so transport errors
	// are retried rather than ending the loop. Only settled requests count:
	// `isError` is cleared while a refetch is pending, so counting it directly
	// would reset the streak on every poll.
	const pollErrors = useRef(0);
	const wasFetching = useRef(false);
	useEffect(() => {
		const settled = wasFetching.current && !isFetching;
		wasFetching.current = isFetching;
		pollErrors.current = nextPollErrorCount(
			settled,
			isError,
			pollErrors.current,
		);
		if (
			!shouldKeepPollingClipWaveform(
				data?.progress,
				pollErrors.current,
				Boolean(data?.error),
			)
		) {
			setGenerating(false);
		}
	}, [data?.error, data?.progress, isError, isFetching]);

	const peaks = useMemo(
		() =>
			data?.data ? decodeWaveformPeaks(data.data) : EMPTY_WAVEFORM_ENVELOPE,
		[data?.data],
	);

	const progress = data?.progress ?? 0;
	// While the poller is still retrying, show progress instead of the error
	// panel; only a stopped poller reports the waveform as unavailable.
	const waveformError = !generating && (isError || Boolean(data?.error));
	const playhead =
		props.durationSeconds > 0
			? Math.min(
					100,
					Math.max(0, (props.positionSeconds / props.durationSeconds) * 100),
				)
			: 0;

	return (
		<div className="relative my-4 h-35 rounded-[1px] overflow-hidden bg-purple-500/18">
			{generating && !waveformError && (
				<div className="absolute top-0 left-0 right-0 z-2 px-2 py-1 bg-slate-900/78 pointer-events-none">
					<span className="text-xs leading-5">
						Building clip waveform ({progress}%)
					</span>
					<ProgressBar value={progress} />
				</div>
			)}
			{!data?.data && !generating && !waveformError && (
				<div className="absolute inset-0 grid place-items-center pointer-events-none">
					<span className="text-muted text-xs leading-5">
						Clip waveform has not been built.
					</span>
				</div>
			)}
			{waveformError && (
				<div className="absolute inset-0 grid place-items-center">
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
					className="absolute right-2 bottom-2 z-3"
					variant="primary"
					size="sm"
					isDisabled={generating}
					onPress={() => {
						pollErrors.current = 0;
						wasFetching.current = false;
						setGenerating(true);
						setRequestKey(Date.now());
					}}
				>
					{waveformError ? "Retry waveform" : "Build waveform"}
				</Button>
			)}
			<div
				className="absolute top-0 bottom-0 w-0.5 bg-white pointer-events-none"
				style={{ left: `${playhead}%` }}
			/>
		</div>
	);
}
