import { useEffect, useRef } from "react";
import type {
	ChannelMixSourceSegment,
	ChannelMixTrack,
} from "../../app/apiSlice";
import { apiSlice, useGetWaveformByUrlQuery } from "../../app/apiSlice";
import { useJobPolling } from "../../realtime/status";
import { fractionInTarget } from "../../shared/geometry";
import { formatDuration } from "../../utils/formatTime";
import { layoutChannelMixSegment } from "./channelMixWaveform";
import { WaveformCanvas } from "./WaveformCanvas";
import {
	LIVE_WAVEFORM_REFRESH_MS,
	useDecodedPeaks,
	type WaveformEnvelope,
} from "./waveformPeaks";

const TRACK_WAVEFORM_HEIGHT = 54;
const SOURCE_WAVEFORM_HEIGHT = 42;

function useChannelMixSourceWaveform(segment: ChannelMixSourceSegment): {
	peaks: WaveformEnvelope;
	loading: boolean;
	building: boolean;
	progress: number;
	error: boolean;
	/** A live source without peaks: it gets them once it ends. */
	notBuilt: boolean;
} {
	const cached = apiSlice.endpoints.getWaveformByUrl.useQueryState(
		segment.waveform_url,
	);
	const settled = Boolean(
		cached.error ||
			cached.currentData?.error ||
			cached.currentData?.data ||
			cached.currentData?.built === false,
	);
	const jobPolling = useJobPolling(1_000);
	const query = useGetWaveformByUrlQuery(segment.waveform_url, {
		// A live source refreshes as it records; any other polls only while a
		// build is under way. RTK Query shares the request across every
		// rendering of the same physical source.
		pollingInterval: segment.live
			? LIVE_WAVEFORM_REFRESH_MS
			: settled
				? 0
				: jobPolling,
	});
	const { refetch } = query;
	// Once a live source ends, its points regroup, and a source without peaks
	// starts building them.
	const wasLive = useRef(segment.live);
	useEffect(() => {
		if (wasLive.current && !segment.live) void refetch();
		wasLive.current = segment.live;
	}, [refetch, segment.live]);
	const encoded = query.currentData?.data;
	const peaks = useDecodedPeaks(encoded);
	const hasData = Boolean(encoded);
	const notBuilt = query.currentData?.built === false;
	const error = query.isError || Boolean(query.currentData?.error);
	const loading = query.isLoading && !hasData;
	const building =
		!hasData && !error && !notBuilt && Boolean(query.currentData);
	const progress = Math.max(0, Math.min(99, query.currentData?.progress ?? 0));

	return { peaks, loading, building, progress, error, notBuilt };
}

function PlacedSourceWaveform(props: {
	segment: ChannelMixSourceSegment;
	durationMs: number;
	height: number;
	label: string;
}) {
	const waveform = useChannelMixSourceWaveform(props.segment);
	const layout = layoutChannelMixSegment(props.segment, props.durationMs);
	if (!layout) return null;
	return (
		<div
			className="absolute top-0 [border-left:1px_solid_rgba(125,_211,_252,_0.35)] [border-right:1px_solid_rgba(125,_211,_252,_0.35)] overflow-hidden"
			style={{
				left: `${layout.leftFraction * 100}%`,
				width: `${layout.widthFraction * 100}%`,
				height: props.height,
			}}
		>
			{waveform.peaks.min.length > 0 && (
				<WaveformCanvas
					peaks={waveform.peaks}
					height={props.height}
					label={props.label}
					startFraction={layout.startFraction}
					endFraction={layout.endFraction}
					durationMs={props.segment.source_duration_ms}
				/>
			)}
			{waveform.loading && waveform.peaks.min.length === 0 && (
				<span className="text-muted text-xs leading-5 px-2">
					Loading waveform…
				</span>
			)}
			{waveform.building && (
				<span className="text-muted text-xs leading-5 px-2">
					Building waveform ({waveform.progress}%)
				</span>
			)}
			{waveform.notBuilt && waveform.peaks.min.length === 0 && (
				<span className="text-muted text-xs leading-5 px-2">
					Waveform once it ends
				</span>
			)}
			{waveform.error && waveform.peaks.min.length === 0 && (
				<span className="text-danger text-xs leading-5 px-2">
					Waveform unavailable
				</span>
			)}
		</div>
	);
}

function TimelineWaveform(props: {
	segments: readonly ChannelMixSourceSegment[];
	durationMs: number;
	positionMs: number;
	height: number;
	label: string;
	onSeek: (positionMs: number) => void;
}) {
	// Matches the seconds-based range used by the sibling position slider, so
	// assistive tech announces the same scale for both seek controls.
	const durationSec = Math.max(0.001, props.durationMs / 1_000);
	const positionSec = Math.min(
		durationSec,
		Math.max(0, props.positionMs / 1_000),
	);
	return (
		<div
			role="slider"
			aria-label={props.label}
			aria-valuemin={0}
			aria-valuemax={durationSec}
			aria-valuenow={positionSec}
			tabIndex={0}
			onKeyDown={(event) => {
				if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
				event.preventDefault();
				const delta = event.shiftKey ? 1 : 0.1;
				props.onSeek(
					Math.min(
						durationSec,
						Math.max(
							0,
							positionSec + (event.key === "ArrowRight" ? delta : -delta),
						),
					) * 1_000,
				);
			}}
			onClick={(event) => {
				props.onSeek(fractionInTarget(event) * props.durationMs);
			}}
			className="relative rounded-[1px] bg-purple-500/12 overflow-hidden cursor-pointer"
			style={{ height: props.height }}
		>
			{props.segments.map((segment) => (
				<PlacedSourceWaveform
					key={segment.id}
					segment={segment}
					durationMs={props.durationMs}
					height={props.height}
					label={props.label}
				/>
			))}
			<div
				aria-hidden="true"
				className="absolute top-0 bottom-0 [border-left:2px_solid_#f8fafc] pointer-events-none"
				style={{
					left: `${Math.max(0, Math.min(1, props.positionMs / Math.max(1, props.durationMs))) * 100}%`,
				}}
			/>
		</div>
	);
}

export function ChannelMixTrackWaveforms(props: {
	tracks: readonly ChannelMixTrack[];
	durationMs: number;
	positionMs: number;
	showSourceRows: boolean;
	onSeek: (positionMs: number) => void;
}) {
	return (
		<div className="flex flex-col gap-1.5 w-full">
			{props.tracks.map((track) => (
				<div key={track.user_id}>
					<TimelineWaveform
						segments={track.segments}
						durationMs={props.durationMs}
						positionMs={props.positionMs}
						height={TRACK_WAVEFORM_HEIGHT}
						label={`${track.display_name ?? `User ${track.user_id}`} waveform`}
						onSeek={props.onSeek}
					/>
					{props.showSourceRows && (
						<div className="flex flex-col gap-1 mt-1 pl-2">
							{track.segments.map((segment) => (
								<div
									key={segment.id}
									className="flex items-center flex-row gap-2"
								>
									<div className="flex-1">
										<TimelineWaveform
											segments={[segment]}
											durationMs={props.durationMs}
											positionMs={props.positionMs}
											height={SOURCE_WAVEFORM_HEIGHT}
											label={`Fragment ${segment.audio_file_id} waveform`}
											onSeek={props.onSeek}
										/>
									</div>
									<span className="text-muted text-xs leading-5 min-w-28 text-right">
										{formatDuration(segment.start_ms / 1_000)} –{" "}
										{formatDuration(segment.end_ms / 1_000)}
									</span>
								</div>
							))}
						</div>
					)}
				</div>
			))}
		</div>
	);
}
