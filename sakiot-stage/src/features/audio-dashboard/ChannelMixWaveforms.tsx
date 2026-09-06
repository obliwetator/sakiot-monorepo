import { useMemo } from "react";
import type {
	ChannelMixSourceSegment,
	ChannelMixTrack,
} from "../../app/apiSlice";
import { apiSlice, useGetWaveformByUrlQuery } from "../../app/apiSlice";
import { Button } from "../../shared/ui";
import { formatDuration } from "../../utils/formatTime";
import { layoutChannelMixSegment } from "./channelMixWaveform";
import { WaveformCanvas } from "./WaveformCanvas";
import {
	decodeWaveformPeaks,
	EMPTY_WAVEFORM_ENVELOPE,
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
	build: () => void;
} {
	const cached = apiSlice.endpoints.getWaveformByUrl.useQueryState(
		segment.waveform_url,
	);
	const cachedData = cached.currentData?.data;
	const cachedError = Boolean(cached.error || cached.currentData?.error);
	const query = useGetWaveformByUrlQuery(segment.waveform_url, {
		// Both live and finalized sources poll only until their first successful
		// payload. RTK Query shares that one request across every rendering of the
		// same physical source, and a live snapshot stays cached for this page.
		pollingInterval: cachedError || cachedData ? 0 : 1_000,
	});
	const encoded = query.currentData?.data;
	const peaks = useMemo(
		() => (encoded ? decodeWaveformPeaks(encoded) : EMPTY_WAVEFORM_ENVELOPE),
		[encoded],
	);
	const hasData = Boolean(encoded);
	const error = query.isError || Boolean(query.currentData?.error);
	const loading = query.isLoading && !hasData;
	const building = !hasData && !error && Boolean(query.currentData);
	const progress = Math.max(0, Math.min(99, query.currentData?.progress ?? 0));

	const build = () => {
		void query.refetch();
	};

	return { peaks, loading, building, progress, error, build };
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
				/>
			)}
			{waveform.loading && waveform.peaks.min.length === 0 && (
				<span className="text-muted text-xs leading-5 px-2">
					Loading waveform…
				</span>
			)}
			{waveform.peaks.min.length === 0 && (
				<Button
					className="absolute right-1 top-1 [z-index:1]"
					variant="outline"
					size="sm"
					isDisabled={waveform.loading || waveform.building}
					onPress={() => {
						waveform.build();
					}}
				>
					{waveform.building
						? `Building waveform (${waveform.progress}%)`
						: "Build waveform"}
				</Button>
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
	return (
		<div
			onClick={(event) => {
				const bounds = event.currentTarget.getBoundingClientRect();
				const fraction = Math.max(
					0,
					Math.min(
						1,
						(event.clientX - bounds.left) / Math.max(1, bounds.width),
					),
				);
				props.onSeek(fraction * props.durationMs);
			}}
			className="relative [border-radius:1px] [background-color:rgba(168,_85,_247,_0.12)] overflow-hidden [cursor:pointer]"
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
