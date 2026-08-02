import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import { useCallback, useEffect, useRef, useState } from "react";
import { PlaybackControls } from "./PlaybackControls";
import { SessionPlaybackTimeline } from "./SessionPlaybackTimeline";
import { SessionWaveform } from "./SessionWaveform";

const ARROW_SEEK_SECONDS = 5;
const CTRL_ARROW_SEEK_SECONDS = 30;

function shortcutTargetIsInteractive(target: EventTarget | null): boolean {
	return (
		target instanceof HTMLElement &&
		Boolean(
			target.closest(
				'input, textarea, select, button, a, [contenteditable]:not([contenteditable="false"]), [role="slider"]',
			),
		)
	);
}

export function SilenceFreePlayer(props: { sessionId: string; url: string }) {
	const audioRef = useRef<HTMLAudioElement | null>(null);
	const [durationMs, setDurationMs] = useState(0);
	const [positionMs, setPositionMs] = useState(0);
	const [seekPreviewMs, setSeekPreviewMs] = useState<number | null>(null);
	const [playing, setPlaying] = useState(false);
	const [volume, setVolume] = useState(1);
	const [playbackRate, setPlaybackRate] = useState(1);
	const [playbackError, setPlaybackError] = useState<string | null>(null);
	const displayedPositionMs = seekPreviewMs ?? positionMs;

	useEffect(() => {
		const audio = audioRef.current;
		if (!audio) return;
		audio.volume = volume;
	}, [volume]);

	useEffect(() => {
		const audio = audioRef.current;
		if (!audio) return;
		audio.playbackRate = playbackRate;
	}, [playbackRate]);

	const seek = useCallback(
		(nextPositionMs: number) => {
			const audio = audioRef.current;
			if (!audio) return;
			const clamped = Math.min(
				Math.max(0, nextPositionMs),
				Math.max(0, durationMs),
			);
			audio.currentTime = clamped / 1_000;
			setPositionMs(clamped);
		},
		[durationMs],
	);

	const togglePlay = useCallback(() => {
		const audio = audioRef.current;
		if (!audio) return;
		setPlaybackError(null);
		if (!audio.paused) {
			audio.pause();
			return;
		}
		if (durationMs > 0 && audio.currentTime * 1_000 >= durationMs - 20) {
			audio.currentTime = 0;
			setPositionMs(0);
		}
		void audio.play().catch(() => {
			setPlaying(false);
			setPlaybackError("Browser blocked or failed silence-free playback.");
		});
	}, [durationMs]);

	useEffect(() => {
		const handleShortcut = (event: KeyboardEvent) => {
			if (shortcutTargetIsInteractive(event.target)) return;
			if (event.ctrlKey || event.metaKey || event.altKey) {
				if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
			}
			if (event.key === " " || event.code === "Space") {
				if (event.repeat) return;
				event.preventDefault();
				togglePlay();
				return;
			}
			if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
			event.preventDefault();
			const distance =
				event.ctrlKey || event.metaKey
					? CTRL_ARROW_SEEK_SECONDS
					: ARROW_SEEK_SECONDS;
			seek(
				positionMs + (event.key === "ArrowRight" ? 1 : -1) * distance * 1_000,
			);
		};
		window.addEventListener("keydown", handleShortcut);
		return () => window.removeEventListener("keydown", handleShortcut);
	}, [positionMs, seek, togglePlay]);

	return (
		<Box sx={{ py: 1 }}>
			{/* biome-ignore lint/a11y/useMediaCaption: user voice recordings do not have a caption track */}
			<audio
				ref={audioRef}
				crossOrigin="use-credentials"
				preload="metadata"
				src={props.url}
				onLoadedMetadata={(event) => {
					const seconds = event.currentTarget.duration;
					if (Number.isFinite(seconds)) setDurationMs(seconds * 1_000);
				}}
				onDurationChange={(event) => {
					const seconds = event.currentTarget.duration;
					if (Number.isFinite(seconds)) setDurationMs(seconds * 1_000);
				}}
				onTimeUpdate={(event) =>
					setPositionMs(event.currentTarget.currentTime * 1_000)
				}
				onPlay={() => setPlaying(true)}
				onPause={() => setPlaying(false)}
				onEnded={() => setPlaying(false)}
				onError={() => {
					setPlaying(false);
					setPlaybackError("Silence-free audio could not be loaded.");
				}}
				style={{ display: "none" }}
			/>

			<SessionPlaybackTimeline
				positionMs={displayedPositionMs}
				durationMs={durationMs}
				onSeek={seek}
				onSeekPreview={setSeekPreviewMs}
				positionAriaLabel="Silence-free playback position"
				waveform={
					<SessionWaveform
						sessionId={props.sessionId}
						positionMs={displayedPositionMs}
						durationMs={durationMs}
						onSeek={seek}
						silenceFree
					/>
				}
			/>

			<Typography
				variant="caption"
				color="text.secondary"
				sx={{ display: "block", mt: 0.5 }}
			>
				Silence-free playback has compressed timestamps. Clip and event offsets
				remain on the Normal timeline.
			</Typography>

			<PlaybackControls
				playing={playing}
				onTogglePlay={togglePlay}
				volume={volume}
				onVolumeChange={setVolume}
				playbackRate={playbackRate}
				onPlaybackRateChange={setPlaybackRate}
			/>
			{playbackError && (
				<Alert severity="error" sx={{ mt: 1 }}>
					{playbackError}
				</Alert>
			)}
		</Box>
	);
}
