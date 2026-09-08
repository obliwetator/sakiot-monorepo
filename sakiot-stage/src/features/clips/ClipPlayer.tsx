import {
	Scissors as ContentCutIcon,
	Download as DownloadIcon,
	Pencil as EditIcon,
	Pause as PauseIcon,
	Play as PlayArrowIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Link as RouterLink, useNavigate } from "react-router-dom";
import {
	BASE_API_URL,
	type ClipData,
	useRenameClipMutation,
} from "../../app/apiSlice";
import {
	authedFetch,
	refreshForMediaRetry,
	SESSION_EXPIRED_MESSAGE,
} from "../../app/authedFetch";
import { PATH_PREFIX_FOR_LOGGED_USERS } from "../../Constants";
import { BaseDialog } from "../../shared/BaseDialog";
import {
	Badge,
	Button,
	IconButton,
	Notice,
	Slider,
	TextField,
	Tooltip,
	TooltipTrigger,
} from "../../shared/ui";
import { formatDuration } from "../../utils/formatTime";
import {
	playbackShortcutTargetAcceptsText,
	playbackShortcutTargetOwnsArrows,
} from "../audio-dashboard/playbackShortcuts";
import { JamIt } from "../audio-dashboard/RangeSlider/JamIt";
import { ClipWaveform } from "./ClipWaveform";
import { isComposedClip } from "./composedClip";

const ARROW_SEEK_SECONDS = 5;
const CTRL_ARROW_SEEK_SECONDS = 30;

function absoluteMediaUrl(path: string): string {
	return new URL(
		path,
		new URL(BASE_API_URL, window.location.origin),
	).toString();
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
		<div className="min-w-0">
			<span className="text-muted text-xs leading-5">{props.label}</span>
			<p className="text-sm [overflow-wrap:anywhere]">{props.value}</p>
		</div>
	);
}

function RenameClipButton(props: { clip: ClipData }) {
	const [open, setOpen] = useState(false);
	const [name, setName] = useState(props.clip.name ?? "");
	const [error, setError] = useState<string | undefined>();
	const [renameClip, { isLoading }] = useRenameClipMutation();
	const trimmedName = name.trim();
	const nameLength = Array.from(trimmedName).length;
	const unchanged = trimmedName === (props.clip.name ?? "").trim();
	const canSubmit = nameLength > 0 && nameLength <= 255 && !unchanged;

	const handleOpen = () => {
		setName(props.clip.name ?? "");
		setError(undefined);
		setOpen(true);
	};
	const handleClose = () => {
		if (!isLoading) setOpen(false);
	};
	const handleRename = async () => {
		if (!canSubmit) {
			if (nameLength === 0) setError("Enter a clip name.");
			return;
		}
		setError(undefined);
		try {
			await renameClip({
				guild_id: props.clip.guild_id,
				clip_id: props.clip.clip_id,
				name: trimmedName,
			}).unwrap();
			setOpen(false);
		} catch {
			setError("Could not rename the clip. Please try again.");
		}
	};

	return (
		<>
			<TooltipTrigger delay={400}>
				<IconButton aria-label="Rename clip" size="sm" onPress={handleOpen}>
					<EditIcon size={16} />
				</IconButton>
				<Tooltip>{"Rename clip"}</Tooltip>
			</TooltipTrigger>
			<BaseDialog
				open={open}
				onClose={handleClose}
				title="Rename clip"
				error={error}
				busy={isLoading}
				actions={
					<>
						<Button
							variant="primary"
							isDisabled={isLoading}
							onPress={handleClose}
						>
							Cancel
						</Button>
						<Button
							variant="primary"
							isDisabled={isLoading || !canSubmit}
							onPress={() => void handleRename()}
						>
							{isLoading ? "Saving..." : "Save"}
						</Button>
					</>
				}
			>
				<p className="text-sm leading-6 text-slate-200">
					Enter a new name for this clip.
				</p>
				<TextField
					value={name}
					onKeyDown={(event) => {
						if (event.key === "Enter" && canSubmit && !isLoading) {
							event.preventDefault();
							void handleRename();
						}
					}}
					autoFocus
					label="Clip name"
					autoComplete="off"
					isDisabled={isLoading}
					onChange={(value) => setName(value)}
					maxLength={255}
				/>
			</BaseDialog>
		</>
	);
}

export function ClipPlayer(props: {
	clip: ClipData;
	absoluteStartMs: number | null;
	canRename: boolean;
}) {
	const navigate = useNavigate();
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
		// One refresh-retry per load: media elements can't report the HTTP
		// status, so a stale access token looks like a plain load failure.
		let refreshed = false;
		audio.crossOrigin = "use-credentials";
		audio.preload = "auto";
		audio.volume = volumeRef.current;
		audio.playbackRate = playbackRateRef.current;
		const mediaPath = `audio/clips/${props.clip.guild_id}/${encodeURIComponent(props.clip.clip_id)}`;
		const loadMedia = (cacheBust: boolean) => {
			audio.src = absoluteMediaUrl(
				cacheBust ? `${mediaPath}?t=${Date.now()}` : mediaPath,
			);
			if (cacheBust) audio.load();
		};
		loadMedia(false);
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
			refreshed = false;
			setReady(true);
		};
		const onError = () => {
			if (!active) return;
			if (refreshed) {
				setError("Clip audio could not be loaded.");
				return;
			}
			refreshed = true;
			void refreshForMediaRetry().then((ok) => {
				if (!active) return;
				if (ok) {
					setError(null);
					loadMedia(true);
				} else {
					setError(SESSION_EXPIRED_MESSAGE);
				}
			});
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
			if (playbackShortcutTargetAcceptsText(event.target)) return;
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
			if (playbackShortcutTargetOwnsArrows(event.target)) return;
			event.preventDefault();
			const distance =
				event.ctrlKey || event.metaKey
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
	const sourceSessionPath = props.clip.recording_session_id
		? `${PATH_PREFIX_FOR_LOGGED_USERS}/${props.clip.guild_id}/audio/session/${encodeURIComponent(props.clip.recording_session_id)}?${new URLSearchParams(
				{
					t: String(props.clip.start_time),
					...(props.clip.silence_free ? { timeline: "silence-free" } : {}),
				},
			)}`
		: null;

	return (
		<div className="px-2 min-[900px]:px-6 pb-8">
			<div className="rounded-md border border-ui-border bg-surface text-fg shadow-none p-4 min-[900px]:p-6 mb-4 [background:linear-gradient(135deg,_rgba(168,85,247,0.14),_rgba(217,70,239,0.04))]">
				<div className="flex justify-between gap-2 flex-col min-[600px]:flex-row">
					<div className="min-w-0">
						<div className="flex items-center flex-row gap-1">
							<h5 className="font-semibold tracking-tight text-2xl min-w-0 [overflow-wrap:anywhere]">
								{props.clip.name || "Unnamed clip"}
							</h5>
							{props.canRename && <RenameClipButton clip={props.clip} />}
						</div>
						<p className="text-muted text-sm">Clip {props.clip.clip_id}</p>
					</div>
					<div className="flex flex-wrap flex-row gap-2">
						<Badge tone={"creative"}>Clip</Badge>
						<Badge>{formatDuration(duration)}</Badge>
						<Badge>{formatClipSize(props.clip.size)}</Badge>
					</div>
				</div>

				<div className="grid [grid-template-columns:1fr] min-[600px]:[grid-template-columns:repeat(2,_minmax(0,_1fr))] min-[1200px]:[grid-template-columns:repeat(3,_minmax(0,_1fr))] gap-4 mt-6">
					<MetadataItem label="Created by user" value={props.clip.user_id} />
					<MetadataItem
						label="Recorded channel"
						value={props.clip.channel_id}
					/>
					<MetadataItem
						label={
							props.clip.silence_free
								? "Silence-free source offset"
								: "Source offset"
						}
						value={formatDuration(props.clip.start_time)}
					/>
					<MetadataItem
						label="Source recording"
						value={
							sourceSessionPath ? (
								<RouterLink to={sourceSessionPath}>
									Session {props.clip.recording_session_id} · open at{" "}
									{formatDuration(props.clip.start_time)}
								</RouterLink>
							) : isComposedClip(props.clip) ? (
								"Composition"
							) : (
								props.clip.original_file_name || "Unknown"
							)
						}
					/>
					<MetadataItem
						label="Stored file"
						value={props.clip.saved_file_name || "Unknown"}
					/>
					<MetadataItem label="Guild" value={props.clip.guild_id} />
				</div>
			</div>

			<ClipWaveform
				guildId={props.clip.guild_id}
				clipId={props.clip.clip_id}
				positionSeconds={displayedPosition}
				durationSeconds={duration}
				onSeek={seek}
			/>

			<Slider
				aria-label="Clip playback position"
				step={0.01}
				value={Math.min(duration, displayedPosition)}
				className="transition-none"
				minValue={0}
				maxValue={Math.max(0.001, duration)}
				onChangeEnd={(value) => {
					setSeekPreview(null);
					seek(Number(value));
				}}
				onChange={(value) => setSeekPreview(Number(value))}
			/>

			<div className="flex justify-between flex-col min-[600px]:flex-row gap-1 -mt-2 mb-4">
				<p className="text-sm tabular-nums">
					Clip time {formatDuration(displayedPosition)} /{" "}
					{formatDuration(duration)}
				</p>
				<p className="text-muted text-sm tabular-nums">
					Real time{" "}
					{absoluteTime == null
						? "Unknown"
						: new Date(absoluteTime).toLocaleString()}
				</p>
			</div>

			<div className="flex items-center flex-col min-[900px]:flex-row gap-4">
				<Button variant="primary" isDisabled={!ready} onPress={togglePlay}>
					{playing ? <PauseIcon /> : <PlayArrowIcon />}
					{playing ? "Pause" : "Play"}
				</Button>
				<div className="min-w-45 flex-1 w-full">
					<span className="text-xs leading-5">Volume</span>
					<Slider
						step={0.05}
						value={volume}
						minValue={0}
						maxValue={1}
						onChange={(value) => setVolume(Number(value))}
					/>
				</div>
				<div className="min-w-45 flex-1 w-full">
					<span className="text-xs leading-5">
						Speed {playbackRate.toFixed(2)}×
					</span>
					<Slider
						step={0.25}
						value={playbackRate}
						minValue={0.5}
						maxValue={2}
						onChange={(value) => setPlaybackRate(Number(value))}
					/>
				</div>
			</div>

			<div className="rounded-md border border-ui-border bg-surface text-fg shadow-none p-4 mt-4">
				<h6 className="font-medium tracking-[0.001em] text-xl mb-2">
					Clip actions
				</h6>
				<div className="flex flex-wrap flex-col min-[600px]:flex-row gap-2">
					<Button variant="outline" onPress={() => void download()}>
						<DownloadIcon />
						Download clip
					</Button>
					<Button
						variant="outline"
						onPress={() =>
							navigate(
								`${PATH_PREFIX_FOR_LOGGED_USERS}/${props.clip.guild_id}/clips/editor?source=${encodeURIComponent(props.clip.clip_id)}`,
							)
						}
					>
						<ContentCutIcon />
						Edit clip
					</Button>
					<Button variant="outline" isDisabled={true}>
						Create clip
					</Button>
					<JamIt visible={true} />
				</div>
				<span className="text-muted text-xs leading-5 block mt-2">
					Clip creation is disabled because this audio is already a clip.
				</span>
			</div>

			{error && (
				<Notice className="mt-4" tone={"error"} announce="alert">
					{error}
				</Notice>
			)}
		</div>
	);
}
