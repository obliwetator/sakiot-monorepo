import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
	ChannelMixParticipantSettings,
	ChannelMixResponse,
	ChannelMixScope,
} from "../../app/apiSlice";
import { BASE_API_URL } from "../../app/apiSlice";
import { authedFetch } from "../../app/authedFetch";
import {
	Alert,
	Box,
	Button,
	Chip,
	Dialog,
	DialogActions,
	DialogContent,
	DialogTitle,
	Divider,
	FormControl,
	FormControlLabel,
	InputLabel,
	LinearProgress,
	MenuItem,
	Paper,
	Select,
	Slider,
	Stack,
	Switch,
	Typography,
} from "../../shared/ui";
import { formatDuration } from "../../utils/formatTime";
import { ChannelMixTrackWaveforms } from "./ChannelMixWaveforms";
import { channelMixRenderSettingsEqual } from "./channelMixDrafts";
import type { ChannelMixOptions } from "./channelMixPreferences";
import {
	CHANNEL_MIX_MAX_GAIN_DB,
	CHANNEL_MIX_MIN_GAIN_DB,
	clampChannelMixGain,
} from "./channelMixState";
import { PlaybackControls } from "./PlaybackControls";
import type { PlaybackShortcutTarget } from "./playbackShortcuts";
import { SessionPlaybackTimeline } from "./SessionPlaybackTimeline";
import { useChannelMixPlayback } from "./useChannelMixPlayback";
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
	settings: ChannelMixParticipantSettings[];
	onSettingsChange: (settings: ChannelMixParticipantSettings[]) => void;
	options: ChannelMixOptions;
	onOptionsChange: (
		update:
			| ChannelMixOptions
			| ((current: ChannelMixOptions) => ChannelMixOptions),
	) => void;
	dialogOpen: boolean;
	onDialogOpenChange: (open: boolean) => void;
	volume: number;
	playbackRate: number;
	onVolumeChange: (volume: number) => void;
	onPlaybackRateChange: (rate: number) => void;
	onBeforePlay: () => void;
	onRegisterStop: (stop: () => void) => void;
	onPlaybackUse: (target: PlaybackShortcutTarget) => void;
	onPlaybackClear: (targetId: string) => void;
	onGenerate?: () => void;
}) {
	const [previewSeekPreviewMs, setPreviewSeekPreviewMs] = useState<
		number | null
	>(null);
	const [downloadError, setDownloadError] = useState<string | null>(null);
	const [generatedPosition, setGeneratedPosition] = useState(0);
	const generatedMediaUrl = props.mix.media_url
		? new URL(
				props.mix.media_url,
				new URL(BASE_API_URL, window.location.origin),
			).toString()
		: null;
	const generated = useSilenceFreePlayback({
		mediaUrl: generatedMediaUrl,
		initialDurationMs: props.mix.duration_ms,
		volume: props.volume,
		playbackRate: props.playbackRate,
		onLoopDisabled: () => {},
	});
	const preview = useChannelMixPlayback({
		tracks: props.mix.tracks,
		durationMs: props.mix.duration_ms,
		settings: props.settings,
		volume: props.volume,
		playbackRate: props.playbackRate,
	});
	const previewRef = useRef(preview);
	const generatedRef = useRef(generated);
	const activeMixModeRef = useRef<"preview" | "generated">("preview");
	const onBeforePlayRef = useRef(props.onBeforePlay);
	const durationRef = useRef(props.mix.duration_ms);
	previewRef.current = preview;
	generatedRef.current = generated;
	onBeforePlayRef.current = props.onBeforePlay;
	durationRef.current = props.mix.duration_ms;
	const shortcutTargetId = `channel-mix:${props.sessionId}`;
	const registerPlayback = useCallback(() => {
		props.onPlaybackUse({
			id: shortcutTargetId,
			toggle: () => {
				if (activeMixModeRef.current === "generated") {
					const current = generatedRef.current;
					if (!current.playing) {
						previewRef.current.stop();
						onBeforePlayRef.current();
					}
					current.togglePlay([0, durationRef.current], false);
					return;
				}
				const current = previewRef.current;
				if (!current.playing) {
					generatedRef.current.stop();
					onBeforePlayRef.current();
				}
				current.togglePlay();
			},
			seek: (positionMs) => {
				if (activeMixModeRef.current === "generated") {
					generatedRef.current.seek(
						positionMs,
						[0, durationRef.current],
						false,
					);
					return;
				}
				previewRef.current.seek(positionMs);
			},
			position: () =>
				activeMixModeRef.current === "generated"
					? generatedRef.current.positionMs
					: previewRef.current.positionMs,
		});
	}, [props.onPlaybackUse, shortcutTargetId]);

	useEffect(
		() => () => props.onPlaybackClear(shortcutTargetId),
		[props.onPlaybackClear, shortcutTargetId],
	);

	useEffect(() => {
		props.onRegisterStop(() => {
			preview.stop();
			generated.stop();
		});
		return () => props.onRegisterStop(() => {});
	}, [generated.stop, preview.stop, props.onRegisterStop]);

	useEffect(() => {
		setGeneratedPosition(generated.positionMs);
	}, [generated.positionMs]);

	const settingByUser = useMemo(
		() => new Map(props.settings.map((setting) => [setting.user_id, setting])),
		[props.settings],
	);
	const hasLiveSources = useMemo(
		() =>
			props.mix.tracks.some((track) =>
				track.segments.some((segment) => segment.live),
			),
		[props.mix.tracks],
	);
	const renderedSettings = props.mix.generation_settings?.participants;
	const renderedOutdated =
		props.mix.status === "ready" &&
		Boolean(renderedSettings) &&
		!channelMixRenderSettingsEqual(props.settings, renderedSettings ?? []);

	const updateSetting = (
		userId: string,
		update: Partial<ChannelMixParticipantSettings>,
	) => {
		props.onSettingsChange(
			props.settings.map((setting) =>
				setting.user_id === userId ? { ...setting, ...update } : setting,
			),
		);
	};

	const togglePreview = () => {
		activeMixModeRef.current = "preview";
		registerPlayback();
		const current = previewRef.current;
		if (!current.playing) {
			generatedRef.current.stop();
			props.onBeforePlay();
		}
		current.togglePlay();
	};
	const seekPreview = (positionMs: number) => {
		setPreviewSeekPreviewMs(null);
		activeMixModeRef.current = "preview";
		registerPlayback();
		previewRef.current.seek(positionMs);
	};
	const toggleGenerated = () => {
		activeMixModeRef.current = "generated";
		registerPlayback();
		const current = generatedRef.current;
		if (!current.playing) {
			previewRef.current.stop();
			props.onBeforePlay();
		}
		current.togglePlay([0, durationRef.current], false);
	};
	const seekGenerated = (positionMs: number) => {
		activeMixModeRef.current = "generated";
		registerPlayback();
		generatedRef.current.seek(positionMs, [0, durationRef.current], false);
	};
	const goLive = () => {
		activeMixModeRef.current = "preview";
		registerPlayback();
		generatedRef.current.stop();
		props.onBeforePlay();
		previewRef.current.goLive();
	};

	const download = async () => {
		setDownloadError(null);
		try {
			const response = await authedFetch(
				`audio/sessions/${props.sessionId}/channel-mix/media?download=true&scope=${props.mix.scope}`,
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
				<Box>
					<Typography variant="h6">Channel mix preview</Typography>
					<Typography variant="caption" color="text.secondary">
						{formatDuration(props.mix.duration_ms / 1_000)} · common timeline ·
						live sources stay preview-only
					</Typography>
				</Box>
				{props.mix.status === "ready" && (
					<Button variant="outlined" onClick={() => void download()}>
						Download rendered mix
					</Button>
				)}
			</Stack>

			<Stack spacing={1} sx={{ mt: 1.5 }}>
				{props.mix.tracks.map((track) => {
					const setting = settingByUser.get(track.user_id) ?? {
						user_id: track.user_id,
						gain_db: 0,
						muted: false,
					};
					return (
						<Stack
							key={track.user_id}
							direction={{ xs: "column", md: "row" }}
							spacing={1}
							alignItems={{ xs: "stretch", md: "center" }}
						>
							<Stack
								direction="row"
								spacing={0.75}
								alignItems="center"
								sx={{ minWidth: { md: 220 } }}
							>
								<Typography sx={{ minWidth: 90 }}>
									{track.display_name ?? `User ${track.user_id}`}
								</Typography>
								{track.is_anchor && <Chip size="small" label="Anchor" />}
								<Button
									size="small"
									variant={setting.muted ? "contained" : "outlined"}
									onClick={() =>
										updateSetting(track.user_id, { muted: !setting.muted })
									}
								>
									{setting.muted ? "Unmute" : "Mute"}
								</Button>
							</Stack>
							<Box sx={{ flex: 1, minWidth: 180 }}>
								<Typography variant="caption">
									Gain {setting.gain_db.toFixed(1)} dB
								</Typography>
								<Slider
									aria-label={`${track.display_name ?? `User ${track.user_id}`} gain`}
									min={CHANNEL_MIX_MIN_GAIN_DB}
									max={CHANNEL_MIX_MAX_GAIN_DB}
									step={0.5}
									value={setting.gain_db}
									onChange={(_event, value) =>
										updateSetting(track.user_id, {
											gain_db: clampChannelMixGain(Number(value)),
										})
									}
								/>
							</Box>
						</Stack>
					);
				})}
			</Stack>

			<SessionPlaybackTimeline
				waveform={
					<ChannelMixTrackWaveforms
						tracks={props.mix.tracks}
						durationMs={props.mix.duration_ms}
						positionMs={previewSeekPreviewMs ?? preview.positionMs}
						showSourceRows={props.options.showSourceRows}
						onSeek={seekPreview}
					/>
				}
				positionMs={previewSeekPreviewMs ?? preview.positionMs}
				durationMs={props.mix.duration_ms}
				onSeek={seekPreview}
				onSeekPreview={setPreviewSeekPreviewMs}
				positionAriaLabel="Channel mix preview position"
			/>
			<PlaybackControls
				playing={preview.playing}
				onTogglePlay={togglePreview}
				volume={props.volume}
				onVolumeChange={props.onVolumeChange}
				playbackRate={props.playbackRate}
				onPlaybackRateChange={props.onPlaybackRateChange}
			/>
			{hasLiveSources && (
				<Stack direction="row" spacing={1} alignItems="center" sx={{ mt: 1 }}>
					<Button
						size="small"
						variant={preview.followingLive ? "contained" : "outlined"}
						onClick={goLive}
					>
						{preview.followingLive ? "Following live" : "Go live"}
					</Button>
					<Typography variant="caption" color="text.secondary">
						Starts two seconds behind the newest common HLS edge. Seeking or
						pausing exits live-follow.
					</Typography>
				</Stack>
			)}
			{Object.entries(preview.sourceErrors).map(([segmentId, message]) => (
				<Alert key={segmentId} severity="error" sx={{ mt: 0.75 }}>
					Source {segmentId}: {message}
				</Alert>
			))}

			{renderedOutdated && (
				<Alert severity="warning" sx={{ mt: 1.5 }}>
					The rendered version uses older participant settings.
					{props.onGenerate && (
						<Button size="small" onClick={props.onGenerate} sx={{ ml: 1 }}>
							Regenerate
						</Button>
					)}
				</Alert>
			)}

			{generatedMediaUrl && props.mix.status === "ready" && (
				<>
					<Divider sx={{ my: 2 }} />
					<Typography variant="subtitle1">Rendered version</Typography>
					<Typography variant="caption" color="text.secondary">
						Server-rendered 48 kHz mono Ogg/Opus artifact
					</Typography>
					<SessionPlaybackTimeline
						waveform={
							<Typography variant="body2" color="text.secondary">
								Final limiter output
							</Typography>
						}
						positionMs={generatedPosition}
						durationMs={props.mix.duration_ms}
						onSeek={seekGenerated}
						onSeekPreview={() => {}}
						positionAriaLabel="Rendered channel mix position"
					/>
					<PlaybackControls
						playing={generated.playing}
						onTogglePlay={toggleGenerated}
						volume={props.volume}
						onVolumeChange={props.onVolumeChange}
						playbackRate={props.playbackRate}
						onPlaybackRateChange={props.onPlaybackRateChange}
					/>
					{/* The element is kept in the DOM so browsers can stream the artifact. */}
					{/* biome-ignore lint/a11y/useMediaCaption: voice recordings have no caption track */}
					<audio
						key={`${generatedMediaUrl}#${generated.retryKey}`}
						ref={generated.audioRef}
						crossOrigin="use-credentials"
						preload="metadata"
						src={generatedMediaUrl}
						onLoadedMetadata={(event) =>
							generated.mediaHandlers.onLoadedMetadata(event.currentTarget)
						}
						onDurationChange={(event) =>
							generated.mediaHandlers.onDurationChange(event.currentTarget)
						}
						onTimeUpdate={(event) =>
							generated.mediaHandlers.onTimeUpdate(event.currentTarget)
						}
						onPlay={generated.mediaHandlers.onPlay}
						onPause={generated.mediaHandlers.onPause}
						onEnded={generated.mediaHandlers.onEnded}
						onError={generated.mediaHandlers.onError}
						style={{ display: "none" }}
					/>
					{generated.playbackError && (
						<Alert severity="error" sx={{ mt: 1 }}>
							{generated.playbackError}
						</Alert>
					)}
				</>
			)}
			{downloadError && (
				<Alert severity="error" sx={{ mt: 1 }}>
					{downloadError}
				</Alert>
			)}

			<Dialog
				open={props.dialogOpen}
				onClose={() => props.onDialogOpenChange(false)}
			>
				<DialogTitle>Channel mix options</DialogTitle>
				<DialogContent>
					<FormControlLabel
						control={
							<Switch
								checked={props.options.showSourceRows}
								onChange={(_event, checked) =>
									props.onOptionsChange((current) => ({
										...current,
										showSourceRows: checked,
									}))
								}
							/>
						}
						label="Show physical source rows"
					/>
					<FormControl fullWidth size="small" sx={{ mt: 1.5 }}>
						<InputLabel id="channel-mix-scope-label">Timeline scope</InputLabel>
						<Select
							labelId="channel-mix-scope-label"
							label="Timeline scope"
							value={props.options.scope}
							onChange={(event) =>
								props.onOptionsChange((current) => ({
									...current,
									scope: event.target.value as ChannelMixScope,
								}))
							}
						>
							<MenuItem value="all_recordings">
								All recordings while connected
							</MenuItem>
							<MenuItem value="selected_session">
								Selected session only (anchor-style)
							</MenuItem>
						</Select>
					</FormControl>
					<Typography variant="caption" display="block" color="text.secondary">
						All recordings is the default. Open this dialog with Ctrl/Cmd+,.
					</Typography>
				</DialogContent>
				<DialogActions>
					<Button onClick={() => props.onDialogOpenChange(false)}>Close</Button>
				</DialogActions>
			</Dialog>
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
