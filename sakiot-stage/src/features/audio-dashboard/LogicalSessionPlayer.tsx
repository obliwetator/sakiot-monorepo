import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import LinearProgress from "@mui/material/LinearProgress";
import Paper from "@mui/material/Paper";
import Slider from "@mui/material/Slider";
import Stack from "@mui/material/Stack";
import Tab from "@mui/material/Tab";
import Tabs from "@mui/material/Tabs";
import TextField from "@mui/material/TextField";
import Typography from "@mui/material/Typography";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useLocation } from "react-router-dom";
import {
	useCreateSessionClipMutation,
	useGenerateSessionChannelMixMutation,
	useGetSessionChannelMixQuery,
	useGetSessionManifestQuery,
} from "../../app/apiSlice";
import { formatDuration } from "../../utils/formatTime";
import { AudioEventTimeline } from "./AudioEventTimeline";
import { ChannelMixPlayer, ChannelMixProgress } from "./ChannelMixPlayer";
import { ClipRangeEditor } from "./ClipRangeEditor";
import { channelMixRenderSettingsEqual } from "./channelMixDrafts";
import { useChannelMixPreferences } from "./channelMixPreferences";
import {
	canGenerateChannelMix,
	channelMixPollInterval,
	parseChannelMixStatus,
} from "./channelMixState";
import {
	LogicalSessionSummary,
	PhysicalRecordingsPanel,
	PlaybackActionsPanel,
	SessionClipEditorPanel,
} from "./LogicalSessionPanels";
import { parseSessionDeepLink } from "./logicalSessionPlaybackState";
import { isValidClipSelection } from "./logicalSessionSelection";
import {
	isolateSessionChannel,
	normalizeSessionSegments,
} from "./logicalSessionTimeline";
import { PlaybackControls } from "./PlaybackControls";
import type { PlaybackShortcutTarget } from "./playbackShortcuts";
import { SessionPlaybackTimeline } from "./SessionPlaybackTimeline";
import { SessionWaveform } from "./SessionWaveform";
import { SilenceFreePlayer } from "./SilenceFreePlayer";
import { useChannelMixDraft } from "./useChannelMixDraft";
import { useSegmentedSessionPlayback } from "./useSegmentedSessionPlayback";
import { useSessionSelectionController } from "./useSessionSelectionController";
import { useSilenceFreePlayback } from "./useSilenceFreePlayback";
import { useSilenceRemoval } from "./useSilenceRemoval";

export function LogicalSessionPlayer(props: { sessionId: string }) {
	const channelMixPreferences = useChannelMixPreferences();
	const channelMixScope = channelMixPreferences.options.scope;
	const location = useLocation();
	const deepLink = useMemo(
		() => parseSessionDeepLink(location.search),
		[location.search],
	);
	const deepLinkedPositionMs = deepLink?.positionMs ?? null;
	const stampClipRequested = deepLink?.fromStamp === true;
	const [finalizedSessionId, setFinalizedSessionId] = useState<string | null>(
		null,
	);
	const {
		data: manifest,
		isLoading,
		isError,
	} = useGetSessionManifestQuery(props.sessionId, {
		pollingInterval: finalizedSessionId === props.sessionId ? 0 : 5_000,
		refetchOnMountOrArgChange: true,
	});
	useEffect(() => {
		if (manifest?.state === "finalized") setFinalizedSessionId(props.sessionId);
	}, [manifest?.state, props.sessionId]);
	const [channelMixPollingInterval, setChannelMixPollingInterval] = useState(0);
	const [activePlaybackTab, setActivePlaybackTab] = useState<
		"normal" | "silence" | "mix"
	>("normal");
	const {
		currentData: channelMix,
		isError: channelMixError,
		refetch: refetchChannelMix,
	} = useGetSessionChannelMixQuery(
		{
			recording_session_id: props.sessionId,
			scope: channelMixScope,
		},
		{
			pollingInterval: channelMixPollingInterval,
			skip: !manifest,
		},
	);
	const channelMixStatus = channelMix?.status;
	useEffect(() => {
		setChannelMixPollingInterval(channelMixPollInterval(channelMixStatus));
	}, [channelMixStatus]);
	const previousManifestStateRef = useRef<string | null>(null);
	useEffect(() => {
		const state = manifest?.state ?? null;
		if (state === "finalized" && previousManifestStateRef.current !== state) {
			// A live mix deliberately stops polling while the anchor is waiting.
			// Refresh once when the manifest becomes final so it can move to idle or
			// ready without keeping a live page on a tight status loop.
			void refetchChannelMix();
		}
		previousManifestStateRef.current = state;
	}, [manifest?.state, refetchChannelMix]);
	const [generateMix, generateMixState] =
		useGenerateSessionChannelMixMutation();
	const mixDraft = useChannelMixDraft(props.sessionId, channelMix);

	const [selectedChannelId, setSelectedChannelId] = useState<string | null>(
		null,
	);
	const normalizedSegments = useMemo(
		() => (manifest ? normalizeSessionSegments(manifest) : []),
		[manifest],
	);
	const physicalFragments = useMemo(
		() =>
			normalizedSegments.filter(
				(segment) =>
					segment.kind !== "silence" && segment.audio_file_id != null,
			),
		[normalizedSegments],
	);
	const channelIds = useMemo(
		() =>
			Array.from(
				new Set(
					physicalFragments
						.map((fragment) => fragment.channel_id)
						.filter((channelId): channelId is string => Boolean(channelId)),
				),
			),
		[physicalFragments],
	);
	const effectiveChannelId =
		selectedChannelId && channelIds.includes(selectedChannelId)
			? selectedChannelId
			: null;
	const segments = useMemo(
		() => isolateSessionChannel(normalizedSegments, effectiveChannelId),
		[effectiveChannelId, normalizedSegments],
	);
	const displayedFragments = useMemo(
		() =>
			effectiveChannelId
				? physicalFragments.filter(
						(fragment) => fragment.channel_id === effectiveChannelId,
					)
				: physicalFragments,
		[effectiveChannelId, physicalFragments],
	);
	const manifestRecordingSessionId = manifest?.recording_session_id;
	const manifestDurationMs = manifest?.duration_ms;
	const selectionManifest = useMemo(
		() =>
			manifestRecordingSessionId !== undefined &&
			manifestDurationMs !== undefined
				? {
						recordingSessionId: manifestRecordingSessionId,
						durationMs: manifestDurationMs,
					}
				: null,
		[manifestDurationMs, manifestRecordingSessionId],
	);

	const [volume, setVolume] = useState(1);
	const [playbackRate, setPlaybackRate] = useState(1);
	const [actionError, setActionError] = useState<string | null>(null);
	const [channelMixActionError, setChannelMixActionError] = useState<
		string | null
	>(null);
	const loopDisableRef = useRef<() => void>(() => {});
	const mixStopRef = useRef<() => void>(() => {});
	const lastPlaybackRef = useRef<PlaybackShortcutTarget | null>(null);
	const rememberLastPlayback = useCallback((target: PlaybackShortcutTarget) => {
		lastPlaybackRef.current = target;
	}, []);
	const clearLastPlayback = useCallback((targetId: string) => {
		if (lastPlaybackRef.current?.id === targetId) {
			lastPlaybackRef.current = null;
		}
	}, []);
	const registerMixStop = useCallback((stop: () => void) => {
		mixStopRef.current = stop;
	}, []);
	const selectionControllerRef = useRef<{
		selectPlaybackTab: (tab: "normal" | "silence") => void;
	} | null>(null);
	const clipEditorRef = useRef<HTMLDivElement | null>(null);

	const normal = useSegmentedSessionPlayback({
		segments,
		durationMs: manifest?.duration_ms ?? 0,
		volume,
		playbackRate,
		onError: setActionError,
		onLoopDisabled: () => loopDisableRef.current(),
	});
	const removal = useSilenceRemoval({
		sessionId: props.sessionId,
		finalized: manifest?.state === "finalized",
		openWhenReady: deepLink?.silenceFree === true,
		onReady: () => {
			mixStopRef.current();
			selectionControllerRef.current?.selectPlaybackTab("silence");
		},
		onUnavailable: () =>
			selectionControllerRef.current?.selectPlaybackTab("normal"),
		onActionError: setActionError,
	});
	const silence = useSilenceFreePlayback({
		mediaUrl: removal.mediaUrl,
		volume,
		playbackRate,
		onLoopDisabled: () => loopDisableRef.current(),
	});
	const selectionController = useSessionSelectionController({
		sessionId: props.sessionId,
		manifest: selectionManifest,
		deepLink,
		normal,
		silence,
		clipEditorRef,
		loopDisableRef,
		lastPlaybackRef,
	});
	selectionControllerRef.current = selectionController;

	const {
		selection,
		playbackTab,
		loopSelection,
		selectionHint,
		previewing,
		changeSelection,
		resetSelection,
		seekActive,
		toggleActivePreview,
		changeLoopSelection,
		setSelectionEdgeFromPlayhead,
		setNearestEdgeFromPlayhead,
		selectPlaybackTab,
		rememberActivePlayback,
	} = selectionController;
	const showChannelMixTab = Boolean(channelMix?.tracks.length);
	useEffect(() => {
		if (activePlaybackTab === "mix" && !showChannelMixTab) {
			mixStopRef.current();
			setActivePlaybackTab(playbackTab);
			return;
		}
		if (activePlaybackTab !== "mix" && activePlaybackTab !== playbackTab) {
			setActivePlaybackTab(playbackTab);
		}
	}, [activePlaybackTab, playbackTab, showChannelMixTab]);
	const positionMs = normal.positionMs;
	const seekPreviewMs = normal.seekPreviewMs;
	const setSeekPreviewMs = normal.setSeekPreviewMs;
	const playing = normal.playing;
	const silencePositionMs = silence.positionMs;
	const silenceSeekPreviewMs = silence.seekPreviewMs;
	const setSilenceSeekPreviewMs = silence.setSeekPreviewMs;
	const silenceDurationMs = silence.durationMs;
	const silencePlaying = silence.playing;
	const silencePlaybackError = silence.playbackError;
	const silenceRetryKey = silence.retryKey;
	const silenceFreeUrl = removal.mediaUrl;
	const silenceRemoval = removal.status;
	const sessionMessage = removal.message;
	const sessionError = removal.error;
	const sessionAction = removal.action;
	const downloadSession = removal.downloadSession;
	const createSilenceFreeSession = removal.create;
	const downloadSilenceFreeSession = removal.downloadSilenceFree;
	const seek = (position: number) =>
		normal.seek(position, selection, loopSelection);
	const seekSilence = (position: number) =>
		silence.seek(position, selection, loopSelection);
	const togglePlay = () => {
		rememberActivePlayback("normal");
		mixStopRef.current();
		normal.togglePlay(selection, loopSelection);
	};
	const toggleSilencePlay = () => {
		rememberActivePlayback("silence");
		mixStopRef.current();
		silence.togglePlay(selection, loopSelection);
	};
	const toggleActivePreviewWithMix = () => {
		mixStopRef.current();
		toggleActivePreview();
	};
	const selectPlaybackTabWithMix = (tab: "normal" | "silence") => {
		mixStopRef.current();
		selectPlaybackTab(tab);
	};
	const selectPlaybackTabFromTabs = (tab: "normal" | "silence" | "mix") => {
		if (tab === "mix") {
			normal.stop();
			silence.stop();
			setActivePlaybackTab("mix");
			return;
		}
		setActivePlaybackTab(tab);
		selectPlaybackTabWithMix(tab);
	};
	const selectPlaybackChannel = (channelId: string | null) => {
		const next = isolateSessionChannel(normalizedSegments, channelId);
		normal.restartWithSegments(next);
		setSelectedChannelId(channelId);
		normal.setSeekPreviewMs(null);
	};

	const [clipName, setClipName] = useState("");
	const [clipMessage, setClipMessage] = useState<string | null>(null);
	const [clipError, setClipError] = useState<string | null>(null);
	const [createClip, clipState] = useCreateSessionClipMutation();
	const createSelectedClip = async () => {
		setClipError(null);
		setClipMessage(null);
		try {
			const response = await createClip({
				recording_session_id: props.sessionId,
				start: selection[0] / 1_000,
				end: selection[1] / 1_000,
				name: clipName.trim() || undefined,
				silence_free: playbackTab === "silence",
			}).unwrap();
			setClipMessage(`Clip created: ${response.name}`);
			setClipName("");
		} catch {
			setClipError("Clip creation failed. Select 1-20 seconds.");
		}
	};
	const createChannelMix = async () => {
		setChannelMixActionError(null);
		mixStopRef.current();
		normal.stop();
		silence.stop();
		try {
			await generateMix({
				recording_session_id: props.sessionId,
				scope: channelMixScope,
				body: { participants: mixDraft.settings },
			}).unwrap();
			await refetchChannelMix();
		} catch {
			setChannelMixActionError("Channel mix generation failed. Try again.");
		}
	};

	if (isLoading) return <Typography>Loading logical recording…</Typography>;
	if (isError || !manifest) {
		return (
			<Alert severity="error">
				Logical recording unavailable or forbidden.
			</Alert>
		);
	}

	const displayedPositionMs = seekPreviewMs ?? positionMs;
	const activeDurationMs =
		playbackTab === "silence" ? silenceDurationMs : manifest.duration_ms;
	const activeDisplayedPositionMs =
		playbackTab === "silence"
			? (silenceSeekPreviewMs ?? silencePositionMs)
			: displayedPositionMs;
	const currentSegment = segments.find(
		(segment) =>
			playbackTab === "normal" &&
			displayedPositionMs >= segment.start_ms &&
			displayedPositionMs < segment.end_ms,
	);
	const hasChannelJourney = new Set(manifest.channel_journey).size > 1;
	const clipSelectionIsValid = isValidClipSelection(selection);
	const mixStatus = parseChannelMixStatus(channelMix?.status);
	const renderedMixSettings = channelMix?.generation_settings?.participants;
	const mixRenderDirty = Boolean(
		renderedMixSettings &&
			!channelMixRenderSettingsEqual(mixDraft.settings, renderedMixSettings),
	);
	const mixCanGenerate = canGenerateChannelMix(
		mixStatus,
		manifest.state === "finalized",
		channelMix?.can_generate ?? false,
		mixDraft.settings,
		mixRenderDirty,
	);
	const mixProcessing = mixStatus === "processing";

	return (
		<Box sx={{ pb: 4 }}>
			{silenceFreeUrl && (
				// biome-ignore lint/a11y/useMediaCaption: user voice recordings do not have a caption track
				<audio
					key={`${silenceFreeUrl}#${silenceRetryKey}`}
					ref={silence.audioRef}
					crossOrigin="use-credentials"
					preload="metadata"
					src={silenceFreeUrl}
					onLoadedMetadata={(event) =>
						silence.mediaHandlers.onLoadedMetadata(event.currentTarget)
					}
					onDurationChange={(event) =>
						silence.mediaHandlers.onDurationChange(event.currentTarget)
					}
					onTimeUpdate={(event) =>
						silence.mediaHandlers.onTimeUpdate(event.currentTarget)
					}
					onPlay={silence.mediaHandlers.onPlay}
					onPause={silence.mediaHandlers.onPause}
					onEnded={silence.mediaHandlers.onEnded}
					onError={silence.mediaHandlers.onError}
					style={{ display: "none" }}
				/>
			)}
			<LogicalSessionSummary
				sessionId={manifest.recording_session_id}
				state={manifest.state}
				userId={manifest.user_id}
				startedAtMs={manifest.started_at_ms}
				durationMs={manifest.duration_ms}
				physicalCount={physicalFragments.length}
				currentSegment={currentSegment}
			/>

			<PlaybackActionsPanel>
				<Tabs
					value={activePlaybackTab}
					onChange={(_event, value: "normal" | "silence" | "mix") =>
						selectPlaybackTabFromTabs(value)
					}
					sx={{ minHeight: 32, mb: 1 }}
				>
					<Tab label="Normal" value="normal" sx={{ minHeight: 32, py: 0 }} />
					{silenceFreeUrl && (
						<Tab
							label="Silence-free"
							value="silence"
							sx={{ minHeight: 32, py: 0 }}
						/>
					)}
					{showChannelMixTab && (
						<Tab
							label="Channel mix"
							value="mix"
							sx={{ minHeight: 32, py: 0 }}
						/>
					)}
				</Tabs>

				<Box
					sx={{ display: activePlaybackTab === "normal" ? "block" : "none" }}
				>
					<SessionPlaybackTimeline
						waveform={
							<SessionWaveform
								key={props.sessionId}
								sessionId={props.sessionId}
								positionMs={displayedPositionMs}
								durationMs={manifest.duration_ms}
								onSeek={seek}
							/>
						}
						positionMs={displayedPositionMs}
						durationMs={manifest.duration_ms}
						onSeek={seek}
						onSeekPreview={setSeekPreviewMs}
						positionAriaLabel="Logical playback position"
						rightDetail={
							<Typography
								variant="body2"
								color="text.secondary"
								sx={{ fontVariantNumeric: "tabular-nums" }}
							>
								Real time{" "}
								{new Date(
									manifest.started_at_ms + displayedPositionMs,
								).toLocaleString()}
							</Typography>
						}
					>
						<AudioEventTimeline
							events={manifest.events}
							durationMs={manifest.duration_ms}
							positionMs={displayedPositionMs}
							startedAtMs={manifest.started_at_ms}
							onSeek={seek}
						/>
					</SessionPlaybackTimeline>
					<PlaybackControls
						playing={playing}
						onTogglePlay={togglePlay}
						volume={volume}
						onVolumeChange={setVolume}
						playbackRate={playbackRate}
						onPlaybackRateChange={setPlaybackRate}
					/>
				</Box>

				{activePlaybackTab === "silence" && silenceFreeUrl && (
					<SilenceFreePlayer
						sessionId={props.sessionId}
						durationMs={silenceDurationMs}
						positionMs={silencePositionMs}
						seekPreviewMs={silenceSeekPreviewMs}
						playing={silencePlaying}
						volume={volume}
						playbackRate={playbackRate}
						playbackError={silencePlaybackError}
						onSeek={seekSilence}
						onSeekPreview={setSilenceSeekPreviewMs}
						onTogglePlay={toggleSilencePlay}
						onVolumeChange={setVolume}
						onPlaybackRateChange={setPlaybackRate}
					/>
				)}

				<Stack
					direction={{ xs: "column", sm: "row" }}
					spacing={1}
					flexWrap="wrap"
					useFlexGap
					sx={{ mt: 1 }}
				>
					<Button
						variant="outlined"
						onClick={() => void downloadSession()}
						disabled={sessionAction !== null}
					>
						{sessionAction === "download" ? "Preparing…" : "Download session"}
					</Button>
					{!silenceFreeUrl && (
						<Button
							variant="contained"
							onClick={() => void createSilenceFreeSession()}
							disabled={
								sessionAction !== null ||
								silenceRemoval.status === "processing" ||
								manifest.state !== "finalized"
							}
							title={
								manifest.state === "finalized"
									? undefined
									: "Silence removal is available after the recording is finalized"
							}
						>
							{sessionAction === "silence" ||
							silenceRemoval.status === "processing"
								? `Removing silence… ${silenceRemoval.progress}%`
								: "Remove silence"}
						</Button>
					)}
					{silenceFreeUrl && (
						<Button
							variant="outlined"
							onClick={() => void createSilenceFreeSession(true)}
							disabled={
								sessionAction !== null || silenceRemoval.status === "processing"
							}
						>
							{sessionAction === "silence"
								? "Regenerating…"
								: "Regenerate silence-free"}
						</Button>
					)}
					{silenceFreeUrl && (
						<Button
							variant="outlined"
							onClick={() => void downloadSilenceFreeSession()}
							disabled={sessionAction !== null}
						>
							{sessionAction === "silence-download"
								? "Preparing…"
								: "Download silence-free"}
						</Button>
					)}
				</Stack>
				{silenceRemoval.status === "processing" && (
					<Box sx={{ mt: 1, maxWidth: 560 }}>
						<Stack direction="row" justifyContent="space-between" mb={0.5}>
							<Typography variant="body2">Removing silence</Typography>
							<Typography variant="body2">
								{silenceRemoval.progress}%
							</Typography>
						</Stack>
						<LinearProgress
							variant="determinate"
							value={silenceRemoval.progress}
							aria-label="Silence removal progress"
						/>
						<Typography variant="caption" color="text.secondary">
							This can continue in the background; progress resumes if you
							refresh the page.
						</Typography>
					</Box>
				)}
				{sessionMessage && (
					<Alert severity="success" sx={{ mt: 1 }}>
						{sessionMessage}
					</Alert>
				)}
				{(sessionError || actionError) && (
					<Alert severity="error" sx={{ mt: 1 }}>
						{sessionError ?? actionError}
					</Alert>
				)}

				<Box sx={{ display: activePlaybackTab === "mix" ? "block" : "none" }}>
					<Paper variant="outlined" sx={{ p: 2, mt: 1.5 }}>
						<Stack
							direction={{ xs: "column", sm: "row" }}
							justifyContent="space-between"
							alignItems={{ xs: "flex-start", sm: "center" }}
							spacing={1}
						>
							<Box>
								<Typography variant="h6">Channel mix</Typography>
								<Typography variant="body2" color="text.secondary">
									{channelMixScope === "all_recordings"
										? "All recordings while the bot was continuously connected to this channel are shown on one timeline."
										: "Only recordings overlapping this selected session are shown on one timeline (anchor-style)."}
								</Typography>
							</Box>
							<Button
								variant="contained"
								onClick={() => void createChannelMix()}
								disabled={!mixCanGenerate || generateMixState.isLoading}
								title={
									channelMix?.reason?.message ??
									(channelMix?.can_generate === false
										? "Every recording in this mix must be finalized"
										: undefined)
								}
							>
								{generateMixState.isLoading
									? "Starting…"
									: mixRenderDirty || channelMix?.status === "ready"
										? "Regenerate channel mix"
										: channelMix?.status === "failed"
											? "Retry mix"
											: "Generate channel mix"}
							</Button>
						</Stack>

						{channelMixError && !channelMix && (
							<Alert severity="error" sx={{ mt: 1.5 }}>
								Channel mix status is unavailable.{" "}
								<Button size="small" onClick={() => void refetchChannelMix()}>
									Retry status
								</Button>
							</Alert>
						)}
						{channelMix && (
							<>
								{channelMix.reason && channelMix.status !== "ready" && (
									<Alert
										severity={channelMix.status === "failed" ? "error" : "info"}
										sx={{ mt: 1.5 }}
									>
										{channelMix.reason.message}
									</Alert>
								)}
								{mixProcessing && (
									<ChannelMixProgress progress={channelMix.progress} />
								)}
								{channelMix.status === "idle" && (
									<Typography
										variant="body2"
										color="text.secondary"
										sx={{ mt: 1 }}
									>
										{channelMix.source_count} source recordings found. The mix
										is ready to generate.
									</Typography>
								)}
								{channelMix.tracks.length > 0 && (
									<ChannelMixPlayer
										sessionId={props.sessionId}
										mix={channelMix}
										settings={mixDraft.settings}
										onSettingsChange={mixDraft.setSettings}
										options={channelMixPreferences.options}
										onOptionsChange={channelMixPreferences.setOptions}
										dialogOpen={channelMixPreferences.dialogOpen}
										onDialogOpenChange={channelMixPreferences.setDialogOpen}
										volume={volume}
										playbackRate={playbackRate}
										onVolumeChange={setVolume}
										onPlaybackRateChange={setPlaybackRate}
										onBeforePlay={() => {
											normal.stop();
											silence.stop();
										}}
										onRegisterStop={registerMixStop}
										onPlaybackUse={rememberLastPlayback}
										onPlaybackClear={clearLastPlayback}
										onGenerate={
											mixCanGenerate ? () => void createChannelMix() : undefined
										}
									/>
								)}
							</>
						)}
						{channelMixActionError && (
							<Alert severity="error" sx={{ mt: 1 }}>
								{channelMixActionError}
							</Alert>
						)}
					</Paper>
				</Box>
			</PlaybackActionsPanel>

			<SessionClipEditorPanel panelRef={clipEditorRef}>
				<Stack
					direction="row"
					spacing={1}
					alignItems="center"
					flexWrap="wrap"
					sx={{ mb: 1.5 }}
				>
					<Typography>
						{playbackTab === "silence" ? "Silence-free selected" : "Selected"}{" "}
						range: {formatDuration(selection[0] / 1_000)} –{" "}
						{formatDuration(selection[1] / 1_000)}
					</Typography>
					{clipSelectionIsValid && (
						<Chip label="Valid clip duration" color="success" size="small" />
					)}
					{playbackTab === "normal" && stampClipRequested && (
						<Chip label="Drafted from stamp" color="info" size="small" />
					)}
				</Stack>
				{playbackTab === "normal" && stampClipRequested && (
					<Typography variant="caption" color="text.secondary">
						Fine seek: Arrow 0.1s · Shift+Arrow 1s · Ctrl/⌘+Arrow 5s · I/O set
						the left/right edges · E sets the nearest edge.
					</Typography>
				)}

				<ClipRangeEditor
					key={`${props.sessionId}:${playbackTab}:${stampClipRequested ? deepLinkedPositionMs : "session"}`}
					sessionId={props.sessionId}
					durationMs={activeDurationMs}
					selection={selection}
					initialFocusMs={
						playbackTab === "normal" && stampClipRequested
							? (deepLinkedPositionMs ?? undefined)
							: undefined
					}
					onSelectionChange={changeSelection}
					positionMs={activeDisplayedPositionMs}
					onSeek={seekActive}
					onSeekPreview={
						playbackTab === "silence"
							? setSilenceSeekPreviewMs
							: setSeekPreviewMs
					}
					onSetEdgeFromPlayhead={setSelectionEdgeFromPlayhead}
					onSetNearestEdgeFromPlayhead={setNearestEdgeFromPlayhead}
					edgeHint={selectionHint}
					onReset={resetSelection}
					onPreview={toggleActivePreviewWithMix}
					previewing={previewing}
					loop={loopSelection}
					onLoopChange={changeLoopSelection}
					silenceFree={playbackTab === "silence"}
				/>

				{(playbackTab === "silence" || !stampClipRequested) && (
					<Slider
						aria-label={
							playbackTab === "silence"
								? "Silence-free action range"
								: "Logical action range"
						}
						min={0}
						max={Math.max(1, activeDurationMs)}
						step={100}
						value={selection}
						onChange={(_event, value) => {
							if (Array.isArray(value)) changeSelection([value[0], value[1]]);
						}}
						valueLabelDisplay="auto"
						valueLabelFormat={(value) => formatDuration(value / 1_000)}
						disableSwap
					/>
				)}
				<Stack
					direction={{ xs: "column", sm: "row" }}
					spacing={1}
					flexWrap="wrap"
				>
					<TextField
						size="small"
						label="Clip name"
						value={clipName}
						onChange={(event) => setClipName(event.target.value)}
						sx={{
							"& .MuiInputBase-root": { height: 40 },
							"& input": {
								boxSizing: "border-box",
								height: "100%",
								py: 0,
							},
						}}
					/>
					<Button
						variant="contained"
						onClick={() => void createSelectedClip()}
						disabled={clipState.isLoading}
						sx={{ height: 40 }}
					>
						Create clip
					</Button>
				</Stack>
				{clipMessage && (
					<Alert severity="success" sx={{ mt: 2 }}>
						{clipMessage}
					</Alert>
				)}
				{clipError && (
					<Alert severity="error" sx={{ mt: 2 }}>
						{clipError}
					</Alert>
				)}
			</SessionClipEditorPanel>

			<PhysicalRecordingsPanel
				sessionId={props.sessionId}
				fragments={displayedFragments}
				allFragments={physicalFragments}
				channelIds={channelIds}
				effectiveChannelId={effectiveChannelId}
				onSelectChannel={selectPlaybackChannel}
				onSeek={seek}
			/>

			{hasChannelJourney && (
				<Paper sx={{ p: 2 }}>
					<Typography variant="h6">Channel journey</Typography>
					<Typography>{manifest.channel_journey.join(" → ")}</Typography>
				</Paper>
			)}
		</Box>
	);
}
