import { Alert, Box, Typography } from "../../shared/ui";
import { PlaybackControls } from "./PlaybackControls";
import { SessionPlaybackTimeline } from "./SessionPlaybackTimeline";
import { SessionWaveform } from "./SessionWaveform";

/**
 * The silence-free player is intentionally presentational. LogicalSessionPlayer
 * owns its audio element so the session player and Clip window always control
 * the same source, playhead and preview bound.
 */
export function SilenceFreePlayer(props: {
	sessionId: string;
	durationMs: number;
	positionMs: number;
	seekPreviewMs: number | null;
	playing: boolean;
	volume: number;
	playbackRate: number;
	playbackError: string | null;
	onSeek: (positionMs: number) => void;
	onSeekPreview: (positionMs: number | null) => void;
	onTogglePlay: () => void;
	onVolumeChange: (volume: number) => void;
	onPlaybackRateChange: (rate: number) => void;
}) {
	const displayedPositionMs = props.seekPreviewMs ?? props.positionMs;

	return (
		<Box sx={{ py: 1 }}>
			<SessionPlaybackTimeline
				positionMs={displayedPositionMs}
				durationMs={props.durationMs}
				onSeek={props.onSeek}
				onSeekPreview={props.onSeekPreview}
				positionAriaLabel="Silence-free playback position"
				waveform={
					<SessionWaveform
						sessionId={props.sessionId}
						positionMs={displayedPositionMs}
						durationMs={props.durationMs}
						onSeek={props.onSeek}
						silenceFree
					/>
				}
			/>

			<Typography
				variant="caption"
				color="text.secondary"
				sx={{ display: "block", mt: 0.5 }}
			>
				Silence-free playback has compressed timestamps. The Clip window below
				uses this same silence-free timeline while this tab is selected.
			</Typography>

			<PlaybackControls
				playing={props.playing}
				onTogglePlay={props.onTogglePlay}
				volume={props.volume}
				onVolumeChange={props.onVolumeChange}
				playbackRate={props.playbackRate}
				onPlaybackRateChange={props.onPlaybackRateChange}
			/>
			{props.playbackError && (
				<Alert severity="error" sx={{ mt: 1 }}>
					{props.playbackError}
				</Alert>
			)}
		</Box>
	);
}
