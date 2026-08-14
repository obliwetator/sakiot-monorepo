import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Stack from "@mui/material/Stack";
import Typography from "@mui/material/Typography";
import { useMemo } from "react";
import type {
	ChannelMixSourceSegment,
	ChannelMixTrack,
} from "../../app/apiSlice";
import { apiSlice, useGetWaveformByUrlQuery } from "../../app/apiSlice";
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
		<Box
			sx={{
				position: "absolute",
				left: `${layout.leftFraction * 100}%`,
				width: `${layout.widthFraction * 100}%`,
				top: 0,
				height: props.height,
				borderLeft: "1px solid rgba(125, 211, 252, 0.35)",
				borderRight: "1px solid rgba(125, 211, 252, 0.35)",
				overflow: "hidden",
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
				<Typography variant="caption" color="text.secondary" sx={{ px: 1 }}>
					Loading waveform…
				</Typography>
			)}
			{waveform.peaks.min.length === 0 && (
				<Button
					size="small"
					variant="outlined"
					disabled={waveform.loading || waveform.building}
					onClick={(event) => {
						event.stopPropagation();
						waveform.build();
					}}
					sx={{ position: "absolute", right: 4, top: 4, zIndex: 1 }}
				>
					{waveform.building
						? `Building waveform (${waveform.progress}%)`
						: "Build waveform"}
				</Button>
			)}
			{waveform.error && waveform.peaks.min.length === 0 && (
				<Typography variant="caption" color="error" sx={{ px: 1 }}>
					Waveform unavailable
				</Typography>
			)}
		</Box>
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
		<Box
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
			sx={{
				position: "relative",
				height: props.height,
				borderRadius: 1,
				bgcolor: "rgba(168, 85, 247, 0.12)",
				overflow: "hidden",
				cursor: "pointer",
			}}
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
			<Box
				aria-hidden="true"
				sx={{
					position: "absolute",
					top: 0,
					bottom: 0,
					left: `${Math.max(0, Math.min(1, props.positionMs / Math.max(1, props.durationMs))) * 100}%`,
					borderLeft: "2px solid #f8fafc",
					pointerEvents: "none",
				}}
			/>
		</Box>
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
		<Stack spacing={0.75} sx={{ width: "100%" }}>
			{props.tracks.map((track) => (
				<Box key={track.user_id}>
					<TimelineWaveform
						segments={track.segments}
						durationMs={props.durationMs}
						positionMs={props.positionMs}
						height={TRACK_WAVEFORM_HEIGHT}
						label={`${track.display_name ?? `User ${track.user_id}`} waveform`}
						onSeek={props.onSeek}
					/>
					{props.showSourceRows && (
						<Stack spacing={0.5} sx={{ mt: 0.5, pl: 1 }}>
							{track.segments.map((segment) => (
								<Stack
									key={segment.id}
									direction="row"
									spacing={1}
									alignItems="center"
								>
									<Box sx={{ flex: 1 }}>
										<TimelineWaveform
											segments={[segment]}
											durationMs={props.durationMs}
											positionMs={props.positionMs}
											height={SOURCE_WAVEFORM_HEIGHT}
											label={`Fragment ${segment.audio_file_id} waveform`}
											onSeek={props.onSeek}
										/>
									</Box>
									<Typography
										variant="caption"
										color="text.secondary"
										sx={{ minWidth: 112, textAlign: "right" }}
									>
										{formatDuration(segment.start_ms / 1_000)} –{" "}
										{formatDuration(segment.end_ms / 1_000)}
									</Typography>
								</Stack>
							))}
						</Stack>
					)}
				</Box>
			))}
		</Stack>
	);
}
