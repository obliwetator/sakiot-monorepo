import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import LinearProgress from "@mui/material/LinearProgress";
import Paper from "@mui/material/Paper";
import Stack from "@mui/material/Stack";
import Typography from "@mui/material/Typography";
import { useEffect, useState } from "react";
import type { ChannelMixResponse } from "../../app/apiSlice";
import { BASE_API_URL } from "../../app/apiSlice";
import { authedFetch } from "../../app/authedFetch";
import { formatDuration } from "../../utils/formatTime";
import { PlaybackControls } from "./PlaybackControls";
import { SessionPlaybackTimeline } from "./SessionPlaybackTimeline";
import { useSilenceFreePlayback } from "./useSilenceFreePlayback";

function saveBlob(blob: Blob, fileName: string) {
	const url = URL.createObjectURL(blob);
	try {
		const anchor = document.createElement("a");
		anchor.href = url;
		anchor.download = fileName;
		document.body.appendChild(anchor);
		anchor.click();
		anchor.remove();
	} finally {
		window.setTimeout(() => URL.revokeObjectURL(url), 1_000);
	}
}

export function ChannelMixPlayer(props: {
	sessionId: string;
	mix: ChannelMixResponse;
	volume: number;
	playbackRate: number;
	onVolumeChange: (volume: number) => void;
	onPlaybackRateChange: (rate: number) => void;
	onBeforePlay: () => void;
	onRegisterStop: (stop: () => void) => void;
}) {
	const mediaUrl = props.mix.media_url
		? new URL(
				props.mix.media_url,
				new URL(BASE_API_URL, window.location.origin),
			).toString()
		: null;
	const playback = useSilenceFreePlayback({
		mediaUrl,
		initialDurationMs: props.mix.duration_ms,
		volume: props.volume,
		playbackRate: props.playbackRate,
		onLoopDisabled: () => {},
	});
	const [downloadError, setDownloadError] = useState<string | null>(null);
	const durationMs = props.mix.duration_ms || playback.durationMs;
	const positionMs = Math.min(
		durationMs,
		playback.seekPreviewMs ?? playback.positionMs,
	);

	useEffect(() => {
		props.onRegisterStop(playback.stop);
		return () => props.onRegisterStop(() => {});
	}, [playback.stop, props.onRegisterStop]);

	const togglePlay = () => {
		if (!playback.playing) props.onBeforePlay();
		playback.togglePlay([0, durationMs], false);
	};
	const seek = (positionMs: number) =>
		playback.seek(positionMs, [0, durationMs], false);
	const download = async () => {
		setDownloadError(null);
		try {
			const response = await authedFetch(
				`audio/sessions/${props.sessionId}/channel-mix/media?download=true`,
			);
			if (!response.ok) {
				setDownloadError(`Download failed (${response.status}).`);
				return;
			}
			saveBlob(
				await response.blob(),
				`session-${props.sessionId}-channel-mix.ogg`,
			);
		} catch {
			setDownloadError("Download failed.");
		}
	};

	return (
		<Paper variant="outlined" sx={{ p: 2, mt: 1.5 }}>
			<Stack
				direction={{ xs: "column", sm: "row" }}
				justifyContent="space-between"
				alignItems={{ xs: "flex-start", sm: "center" }}
				spacing={1}
			>
				<Typography variant="h6">Channel mix player</Typography>
				<Button variant="outlined" onClick={() => void download()}>
					Download mix
				</Button>
			</Stack>
			<Typography variant="caption" color="text.secondary">
				{formatDuration(durationMs / 1_000)} · 48 kHz mono · Ogg/Opus
			</Typography>
			{/* The mixed waveform is intentionally not generated in this iteration. */}
			<SessionPlaybackTimeline
				waveform={
					<Typography variant="body2" color="text.secondary">
						Timestamp-aligned audio
					</Typography>
				}
				positionMs={positionMs}
				durationMs={durationMs}
				onSeek={seek}
				onSeekPreview={playback.setSeekPreviewMs}
				positionAriaLabel="Channel mix playback position"
			/>
			<PlaybackControls
				playing={playback.playing}
				onTogglePlay={togglePlay}
				volume={props.volume}
				onVolumeChange={props.onVolumeChange}
				playbackRate={props.playbackRate}
				onPlaybackRateChange={props.onPlaybackRateChange}
			/>
			{mediaUrl && (
				// biome-ignore lint/a11y/useMediaCaption: user voice recordings do not have a caption track
				<audio
					key={`${mediaUrl}#${playback.retryKey}`}
					ref={playback.audioRef}
					crossOrigin="use-credentials"
					preload="metadata"
					src={mediaUrl}
					onLoadedMetadata={(event) =>
						playback.mediaHandlers.onLoadedMetadata(event.currentTarget)
					}
					onDurationChange={(event) =>
						playback.mediaHandlers.onDurationChange(event.currentTarget)
					}
					onTimeUpdate={(event) =>
						playback.mediaHandlers.onTimeUpdate(event.currentTarget)
					}
					onPlay={playback.mediaHandlers.onPlay}
					onPause={playback.mediaHandlers.onPause}
					onEnded={playback.mediaHandlers.onEnded}
					onError={playback.mediaHandlers.onError}
					style={{ display: "none" }}
				/>
			)}
			{playback.playbackError && (
				<Alert severity="error" sx={{ mt: 1 }}>
					{playback.playbackError}
				</Alert>
			)}
			{downloadError && (
				<Alert severity="error" sx={{ mt: 1 }}>
					{downloadError}
				</Alert>
			)}
		</Paper>
	);
}

export function ChannelMixParticipants(props: {
	participants: ChannelMixResponse["participants"];
}) {
	if (props.participants.length === 0) return null;
	return (
		<Stack
			direction="row"
			spacing={0.75}
			flexWrap="wrap"
			useFlexGap
			sx={{ mt: 1 }}
		>
			{props.participants.map((participant) => (
				<Chip
					key={participant.user_id}
					label={participant.display_name ?? `User ${participant.user_id}`}
					variant="outlined"
					size="small"
				/>
			))}
		</Stack>
	);
}

export function ChannelMixProgress(props: { progress: number }) {
	return (
		<Box sx={{ mt: 1, maxWidth: 560 }}>
			<Stack direction="row" justifyContent="space-between" mb={0.5}>
				<Typography variant="body2">Generating channel mix</Typography>
				<Typography variant="body2">{props.progress}%</Typography>
			</Stack>
			<LinearProgress
				variant="determinate"
				value={Math.max(0, Math.min(99, props.progress))}
				aria-label="Channel mix generation progress"
			/>
		</Box>
	);
}
