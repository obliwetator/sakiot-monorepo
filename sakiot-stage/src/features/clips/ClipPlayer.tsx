import DownloadIcon from "@mui/icons-material/Download";
import PauseIcon from "@mui/icons-material/Pause";
import PlayArrowIcon from "@mui/icons-material/PlayArrow";
import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import Paper from "@mui/material/Paper";
import Slider from "@mui/material/Slider";
import Stack from "@mui/material/Stack";
import Typography from "@mui/material/Typography";
import type { ReactNode } from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import { BASE_API_URL, type ClipData } from "../../app/apiSlice";
import { authedFetch } from "../../app/authedFetch";
import { formatDuration } from "../../utils/formatTime";
import { JamIt } from "../audio-dashboard/RangeSlider/JamIt";
import { ClipWaveform } from "./ClipWaveform";

const ARROW_SEEK_SECONDS = 5;
const CTRL_ARROW_SEEK_SECONDS = 30;

function absoluteMediaUrl(path: string): string {
	return new URL(
		path,
		new URL(BASE_API_URL, window.location.origin),
	).toString();
}

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

function formatClipSize(bytes: number | null | undefined): string {
	if (bytes == null) return "Unknown";
	if (bytes < 1_024 * 1_024) return `${(bytes / 1_024).toFixed(1)} KB`;
	return `${(bytes / (1_024 * 1_024)).toFixed(1)} MB`;
}

function safeFileName(name: string): string {
	const sanitized = name
		.replaceAll(/[^a-zA-Z0-9._-]+/g, "-")
		.replaceAll(/^-+|-+$/g, "");
	return sanitized || "clip";
}

function saveBlob(blob: Blob, fileName: string) {
	const url = URL.createObjectURL(blob);
	const anchor = document.createElement("a");
	anchor.href = url;
	anchor.download = fileName;
	anchor.click();
	URL.revokeObjectURL(url);
}

function MetadataItem(props: { label: string; value: ReactNode }) {
	return (
		<Box sx={{ minWidth: 0 }}>
			<Typography variant="caption" color="text.secondary">
				{props.label}
			</Typography>
			<Typography variant="body2" sx={{ overflowWrap: "anywhere" }}>
				{props.value}
			</Typography>
		</Box>
	);
}

export function ClipPlayer(props: {
	clip: ClipData;
	absoluteStartMs: number | null;
}) {
	const audioRef = useRef<HTMLAudioElement | null>(null);
	const positionRef = useRef(0);
	const durationRef = useRef(props.clip.length ?? 0);
	const playAttemptRef = useRef(0);
	const volumeRef = useRef(1);
	const playbackRateRef = useRef(1);
	const [position, setPosition] = useState(0);
	const [seekPreview, setSeekPreview] = useState<number | null>(null);
	const [duration, setDuration] = useState(props.clip.length ?? 0);
	const [playing, setPlaying] = useState(false);
	const [ready, setReady] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [volume, setVolume] = useState(1);
	const [playbackRate, setPlaybackRate] = useState(1);

	useEffect(() => {
		const audio = new Audio();
		playAttemptRef.current += 1;
		let active = true;
		audio.crossOrigin = "use-credentials";
		audio.preload = "auto";
		audio.volume = volumeRef.current;
		audio.playbackRate = playbackRateRef.current;
		audio.src = absoluteMediaUrl(
			`audio/clips/${props.clip.guild_id}/${encodeURIComponent(props.clip.clip_id)}`,
		);
		audioRef.current = audio;
		positionRef.current = 0;
		durationRef.current = props.clip.length ?? 0;
		setPosition(0);
		setSeekPreview(null);
		setDuration(props.clip.length ?? 0);
		setPlaying(false);
		setReady(false);
		setError(null);

		const updateDuration = () => {
			if (Number.isFinite(audio.duration)) {
				durationRef.current = audio.duration;
				setDuration(audio.duration);
			}
		};
		const updatePosition = () => {
			positionRef.current = audio.currentTime;
			setPosition(audio.currentTime);
		};
		const onCanPlay = () => {
			updateDuration();
			setReady(true);
		};
		const onError = () => {
			if (!active) return;
			setError("Clip audio could not be loaded.");
		};
		const onPlay = () => setPlaying(true);
		const onPause = () => setPlaying(false);

		audio.addEventListener("loadedmetadata", updateDuration);
		audio.addEventListener("durationchange", updateDuration);
		audio.addEventListener("timeupdate", updatePosition);
		audio.addEventListener("canplay", onCanPlay);
		audio.addEventListener("play", onPlay);
		audio.addEventListener("pause", onPause);
		audio.addEventListener("ended", onPause);
		audio.addEventListener("error", onError);

		return () => {
			active = false;
			playAttemptRef.current += 1;
			audio.pause();
			audio.removeAttribute("src");
			audio.load();
			if (audioRef.current === audio) audioRef.current = null;
		};
	}, [props.clip.clip_id, props.clip.guild_id, props.clip.length]);

	useEffect(() => {
		volumeRef.current = volume;
		if (audioRef.current) audioRef.current.volume = volume;
	}, [volume]);

	useEffect(() => {
		playbackRateRef.current = playbackRate;
		if (audioRef.current) audioRef.current.playbackRate = playbackRate;
	}, [playbackRate]);

	const seek = useCallback((seconds: number) => {
		const audio = audioRef.current;
		if (!audio) return;
		const next = Math.max(0, Math.min(seconds, durationRef.current));
		audio.currentTime = next;
		positionRef.current = next;
		setPosition(next);
	}, []);

	const togglePlay = useCallback(() => {
		const audio = audioRef.current;
		if (!audio) return;
		if (!audio.paused) {
			playAttemptRef.current += 1;
			audio.pause();
			return;
		}
		setError(null);
		if (positionRef.current >= durationRef.current) seek(0);
		const attempt = ++playAttemptRef.current;
		void audio.play().catch((playError: unknown) => {
			if (playAttemptRef.current !== attempt) return;
			if (playError instanceof DOMException && playError.name === "AbortError")
				return;
			setError("Browser blocked or failed audio playback.");
			setPlaying(false);
		});
	}, [seek]);

	useEffect(() => {
		const handleShortcut = (event: KeyboardEvent) => {
			if (shortcutTargetIsInteractive(event.target)) return;
			if (event.key === " " || event.code === "Space") {
				if (event.repeat) return;
				event.preventDefault();
				togglePlay();
				return;
			}
			if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
			event.preventDefault();
			const distance = event.ctrlKey
				? CTRL_ARROW_SEEK_SECONDS
				: ARROW_SEEK_SECONDS;
			const direction = event.key === "ArrowRight" ? 1 : -1;
			setSeekPreview(null);
			seek(positionRef.current + direction * distance);
		};
		window.addEventListener("keydown", handleShortcut);
		return () => window.removeEventListener("keydown", handleShortcut);
	}, [seek, togglePlay]);

	const download = async () => {
		setError(null);
		const response = await authedFetch(
			`audio/clips/${props.clip.guild_id}/${encodeURIComponent(props.clip.clip_id)}`,
		);
		if (!response.ok) {
			setError(`Clip download failed (${response.status}).`);
			return;
		}
		saveBlob(
			await response.blob(),
			`${safeFileName(props.clip.name ?? props.clip.clip_id)}.ogg`,
		);
	};

	const displayedPosition = seekPreview ?? position;
	const absoluteTime =
		props.absoluteStartMs == null
			? null
			: props.absoluteStartMs + displayedPosition * 1_000;

	return (
		<Box sx={{ px: { xs: 1, md: 3 }, pb: 4 }}>
			<Paper
				variant="outlined"
				sx={{
					p: { xs: 2, md: 3 },
					mb: 2,
					background:
						"linear-gradient(135deg, rgba(168,85,247,0.14), rgba(217,70,239,0.04))",
				}}
			>
				<Stack
					direction={{ xs: "column", sm: "row" }}
					justifyContent="space-between"
					gap={1}
				>
					<Box sx={{ minWidth: 0 }}>
						<Typography variant="h5" sx={{ overflowWrap: "anywhere" }}>
							{props.clip.name || "Unnamed clip"}
						</Typography>
						<Typography variant="body2" color="text.secondary">
							Clip {props.clip.clip_id}
						</Typography>
					</Box>
					<Stack direction="row" spacing={1} flexWrap="wrap">
						<Chip label="Clip" color="secondary" />
						<Chip label={formatDuration(duration)} />
						<Chip label={formatClipSize(props.clip.size)} />
					</Stack>
				</Stack>

				<Box
					sx={{
						display: "grid",
						gridTemplateColumns: {
							xs: "1fr",
							sm: "repeat(2, minmax(0, 1fr))",
							lg: "repeat(3, minmax(0, 1fr))",
						},
						gap: 2,
						mt: 3,
					}}
				>
					<MetadataItem label="Created by user" value={props.clip.user_id} />
					<MetadataItem
						label="Recorded channel"
						value={props.clip.channel_id}
					/>
					<MetadataItem
						label="Source offset"
						value={formatDuration(props.clip.start_time)}
					/>
					<MetadataItem
						label="Source recording"
						value={props.clip.original_file_name || "Unknown"}
					/>
					<MetadataItem
						label="Stored file"
						value={props.clip.saved_file_name || "Unknown"}
					/>
					<MetadataItem label="Guild" value={props.clip.guild_id} />
				</Box>
			</Paper>

			<ClipWaveform
				guildId={props.clip.guild_id}
				clipId={props.clip.clip_id}
				positionSeconds={displayedPosition}
				durationSeconds={duration}
				onSeek={seek}
			/>

			<Slider
				aria-label="Clip playback position"
				min={0}
				max={Math.max(0.001, duration)}
				step={0.01}
				value={Math.min(duration, displayedPosition)}
				onChange={(_event, value) => setSeekPreview(Number(value))}
				onChangeCommitted={(_event, value) => {
					setSeekPreview(null);
					seek(Number(value));
				}}
				valueLabelDisplay="auto"
				valueLabelFormat={formatDuration}
				sx={{
					"& .MuiSlider-thumb, & .MuiSlider-track": { transition: "none" },
				}}
			/>

			<Stack
				direction={{ xs: "column", sm: "row" }}
				justifyContent="space-between"
				spacing={0.5}
				sx={{ mt: -1, mb: 2 }}
			>
				<Typography variant="body2" sx={{ fontVariantNumeric: "tabular-nums" }}>
					Clip time {formatDuration(displayedPosition)} /{" "}
					{formatDuration(duration)}
				</Typography>
				<Typography
					variant="body2"
					color="text.secondary"
					sx={{ fontVariantNumeric: "tabular-nums" }}
				>
					Real time{" "}
					{absoluteTime == null
						? "Unknown"
						: new Date(absoluteTime).toLocaleString()}
				</Typography>
			</Stack>

			<Stack
				direction={{ xs: "column", md: "row" }}
				spacing={2}
				alignItems="center"
			>
				<Button
					variant="contained"
					onClick={togglePlay}
					disabled={!ready}
					startIcon={playing ? <PauseIcon /> : <PlayArrowIcon />}
				>
					{playing ? "Pause" : "Play"}
				</Button>
				<Box sx={{ minWidth: 180, flex: 1, width: "100%" }}>
					<Typography variant="caption">Volume</Typography>
					<Slider
						min={0}
						max={1}
						step={0.05}
						value={volume}
						onChange={(_event, value) => setVolume(Number(value))}
					/>
				</Box>
				<Box sx={{ minWidth: 180, flex: 1, width: "100%" }}>
					<Typography variant="caption">
						Speed {playbackRate.toFixed(2)}×
					</Typography>
					<Slider
						min={0.5}
						max={2}
						step={0.25}
						value={playbackRate}
						onChange={(_event, value) => setPlaybackRate(Number(value))}
					/>
				</Box>
			</Stack>

			<Paper variant="outlined" sx={{ p: 2, mt: 2 }}>
				<Typography variant="h6" gutterBottom>
					Clip actions
				</Typography>
				<Stack
					direction={{ xs: "column", sm: "row" }}
					spacing={1}
					flexWrap="wrap"
				>
					<Button
						variant="outlined"
						startIcon={<DownloadIcon />}
						onClick={() => void download()}
					>
						Download clip
					</Button>
					<Button variant="outlined" disabled>
						Create clip
					</Button>
					<JamIt visible={true} />
				</Stack>
				<Typography
					variant="caption"
					color="text.secondary"
					sx={{ display: "block", mt: 1 }}
				>
					Clip creation is disabled because this audio is already a clip.
				</Typography>
			</Paper>

			{error && (
				<Alert severity="error" sx={{ mt: 2 }}>
					{error}
				</Alert>
			)}
		</Box>
	);
}
