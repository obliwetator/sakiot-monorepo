import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
	ChannelMixParticipantSettings,
	ChannelMixResponse,
	ChannelMixScope,
} from "../../app/apiSlice";
import { BASE_API_URL } from "../../app/apiSlice";
import { authedFetch } from "../../app/authedFetch";
import {
	Badge,
	Button,
	DialogHeading,
	Modal,
	Notice,
	ProgressBar,
	Select,
	SelectItem,
	Slider,
	Switch,
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
		<div className="rounded-md border border-ui-border bg-surface text-fg shadow-none p-4 mt-3">
			<div className="flex justify-between items-start min-[600px]:items-center flex-col min-[600px]:flex-row gap-2">
				<div>
					<h6 className="font-medium tracking-[0.001em] text-xl">
						Channel mix preview
					</h6>
					<span className="text-muted text-xs leading-5">
						{formatDuration(props.mix.duration_ms / 1_000)} · common timeline ·
						live sources stay preview-only
					</span>
				</div>
				{props.mix.status === "ready" && (
					<Button variant="outline" onPress={() => void download()}>
						Download rendered mix
					</Button>
				)}
			</div>

			<div className="flex flex-col gap-2 mt-3">
				{props.mix.tracks.map((track) => {
					const setting = settingByUser.get(track.user_id) ?? {
						user_id: track.user_id,
						gain_db: 0,
						muted: false,
					};
					return (
						<div
							key={track.user_id}
							className="flex items-stretch min-[900px]:items-center flex-col min-[900px]:flex-row gap-2"
						>
							<div className="flex items-center flex-row gap-1.5 min-[900px]:min-w-55">
								<p className="leading-6 min-w-22.5">
									{track.display_name ?? `User ${track.user_id}`}
								</p>
								{track.is_anchor && <Badge size={"sm"}>Anchor</Badge>}
								<Button
									variant={setting.muted ? "primary" : "outline"}
									size="sm"
									onPress={() =>
										updateSetting(track.user_id, { muted: !setting.muted })
									}
								>
									{setting.muted ? "Unmute" : "Mute"}
								</Button>
							</div>
							<div className="flex-1 min-w-45">
								<span className="text-xs leading-5">
									Gain {setting.gain_db.toFixed(1)} dB
								</span>
								<Slider
									aria-label={`${track.display_name ?? `User ${track.user_id}`} gain`}
									step={0.5}
									value={setting.gain_db}
									minValue={CHANNEL_MIX_MIN_GAIN_DB}
									maxValue={CHANNEL_MIX_MAX_GAIN_DB}
									onChange={(value) =>
										updateSetting(track.user_id, {
											gain_db: clampChannelMixGain(Number(value)),
										})
									}
								/>
							</div>
						</div>
					);
				})}
			</div>

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
				<div className="flex items-center flex-row gap-2 mt-2">
					<Button
						variant={preview.followingLive ? "primary" : "outline"}
						size="sm"
						onPress={goLive}
					>
						{preview.followingLive ? "Following live" : "Go live"}
					</Button>
					<span className="text-muted text-xs leading-5">
						Starts two seconds behind the newest common HLS edge. Seeking or
						pausing exits live-follow.
					</span>
				</div>
			)}
			{Object.entries(preview.sourceErrors).map(([segmentId, message]) => (
				<Notice
					key={segmentId}
					className="mt-1.5"
					tone={"error"}
					announce="alert"
				>
					Source {segmentId}: {message}
				</Notice>
			))}

			{renderedOutdated && (
				<Notice className="mt-3" tone={"warning"} announce="status">
					The rendered version uses older participant settings.
					{props.onGenerate && (
						<Button className="ml-2" size="sm" onPress={props.onGenerate}>
							Regenerate
						</Button>
					)}
				</Notice>
			)}

			{generatedMediaUrl && props.mix.status === "ready" && (
				<>
					<hr className="w-full border-t border-ui-border my-4" />
					<h6 className="text-base">Rendered version</h6>
					<span className="text-muted text-xs leading-5">
						Server-rendered 48 kHz mono Ogg/Opus artifact
					</span>
					<SessionPlaybackTimeline
						waveform={
							<p className="text-muted text-sm">Final limiter output</p>
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
						<Notice className="mt-2" tone={"error"} announce="alert">
							{generated.playbackError}
						</Notice>
					)}
				</>
			)}
			{downloadError && (
				<Notice className="mt-2" tone={"error"} announce="alert">
					{downloadError}
				</Notice>
			)}

			<Modal
				isOpen={props.dialogOpen}
				onOpenChange={(isOpen) => {
					if (!isOpen) props.onDialogOpenChange(false);
				}}
			>
				<DialogHeading>Channel mix options</DialogHeading>
				<div className="space-y-3 px-5 py-4">
					<Switch
						isSelected={props.options.showSourceRows}
						onChange={(checked) =>
							props.onOptionsChange((current) => ({
								...current,
								showSourceRows: checked,
							}))
						}
					>
						Show physical source rows
					</Switch>
					<div className="relative flex min-w-0 w-full mt-3">
						<Select
							label="Timeline scope"
							selectedKey={props.options.scope}
							onSelectionChange={(value) =>
								props.onOptionsChange((current) => ({
									...current,
									scope: value as ChannelMixScope,
								}))
							}
						>
							<SelectItem id={"all_recordings"}>
								All recordings while connected
							</SelectItem>
							<SelectItem id={"selected_session"}>
								Selected session only (anchor-style)
							</SelectItem>
						</Select>
					</div>
					<span className="block text-muted text-xs leading-5">
						All recordings is the default. Open this dialog with Ctrl/Cmd+,.
					</span>
				</div>
				<div className="flex justify-end gap-2 border-t border-ui-border px-5 py-3">
					<Button onPress={() => props.onDialogOpenChange(false)}>Close</Button>
				</div>
			</Modal>
		</div>
	);
}

export function ChannelMixParticipants(props: {
	participants: ChannelMixResponse["participants"];
}) {
	if (props.participants.length === 0) return null;
	return (
		<div className="flex flex-wrap flex-row gap-1.5 mt-2">
			{props.participants.map((participant) => (
				<Badge key={participant.user_id} appearance={"outline"} size={"sm"}>
					{participant.display_name ?? `User ${participant.user_id}`}
				</Badge>
			))}
		</div>
	);
}

export function ChannelMixProgress(props: { progress: number }) {
	return (
		<div className="mt-2 max-w-140">
			<div className="flex justify-between mb-1 flex-row">
				<p className="text-sm">Generating channel mix</p>
				<p className="text-sm">{props.progress}%</p>
			</div>
			<ProgressBar
				value={Math.max(0, Math.min(99, props.progress))}
				aria-label="Channel mix generation progress"
			/>
		</div>
	);
}
