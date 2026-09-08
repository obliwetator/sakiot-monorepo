import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useLocation } from "react-router-dom";
import {
	useCreateSessionClipMutation,
	useGenerateSessionChannelMixMutation,
	useGetSessionChannelMixQuery,
	useGetSessionManifestQuery,
} from "../../app/apiSlice";
import {
	Badge,
	Button,
	Notice,
	ProgressBar,
	Slider,
	Tab,
	TabList,
	TabPanel,
	Tabs,
	TextField,
	Tooltip,
	TooltipTrigger,
} from "../../shared/ui";
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

	if (isLoading) return <p className="leading-6">Loading logical recording…</p>;
	if (isError || !manifest) {
		return (
			<Notice tone={"error"} announce="alert">
				Logical recording unavailable or forbidden.
			</Notice>
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
		<div className="pb-8">
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
					className="min-h-8 mb-2"
					selectedKey={activePlaybackTab}
					onSelectionChange={(value) => {
						if (value === "normal" || value === "silence" || value === "mix")
							selectPlaybackTabFromTabs(value);
					}}
				>
					<TabList aria-label="View">
						<Tab className="min-h-8 py-0" id={"normal"}>
							Normal
						</Tab>
						{silenceFreeUrl && (
							<Tab className="min-h-8 py-0" id={"silence"}>
								Silence-free
							</Tab>
						)}
						{showChannelMixTab && (
							<Tab className="min-h-8 py-0" id={"mix"}>
								Channel mix
							</Tab>
						)}
					</TabList>

					<TabPanel
						id="normal"
						shouldForceMount
						className="data-[inert]:hidden"
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
								<p className="leading-6 text-muted text-sm tabular-nums">
									Real time{" "}
									{new Date(
										manifest.started_at_ms + displayedPositionMs,
									).toLocaleString()}
								</p>
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
					</TabPanel>

					{silenceFreeUrl && (
						<TabPanel id="silence">
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
						</TabPanel>
					)}

					<div className="flex items-center flex-wrap flex-row gap-2 mt-2">
						<Button
							variant="outline"
							isDisabled={sessionAction !== null}
							onPress={() => void downloadSession()}
						>
							{sessionAction === "download" ? "Preparing…" : "Download session"}
						</Button>
						{!silenceFreeUrl && (
							<TooltipTrigger delay={400}>
								<Button
									variant="primary"
									isDisabled={
										sessionAction !== null ||
										silenceRemoval.status === "processing" ||
										manifest.state !== "finalized"
									}
									onPress={() => void createSilenceFreeSession()}
								>
									{sessionAction === "silence" ||
									silenceRemoval.status === "processing"
										? `Removing silence… ${silenceRemoval.progress}%`
										: "Remove silence"}
								</Button>
								<Tooltip>
									{manifest.state === "finalized"
										? undefined
										: "Silence removal is available after the recording is finalized"}
								</Tooltip>
							</TooltipTrigger>
						)}
						{silenceFreeUrl && (
							<Button
								variant="outline"
								isDisabled={
									sessionAction !== null ||
									silenceRemoval.status === "processing"
								}
								onPress={() => void createSilenceFreeSession(true)}
							>
								{sessionAction === "silence"
									? "Regenerating…"
									: "Regenerate silence-free"}
							</Button>
						)}
						{silenceFreeUrl && (
							<Button
								variant="outline"
								isDisabled={sessionAction !== null}
								onPress={() => void downloadSilenceFreeSession()}
							>
								{sessionAction === "silence-download"
									? "Preparing…"
									: "Download silence-free"}
							</Button>
						)}
					</div>
					{silenceRemoval.status === "processing" && (
						<div className="mt-2 max-w-140">
							<div className="flex justify-between mb-1 flex-row">
								<p className="text-sm">Removing silence</p>
								<p className="text-sm">{silenceRemoval.progress}%</p>
							</div>
							<ProgressBar
								value={silenceRemoval.progress}
								aria-label="Silence removal progress"
							/>
							<span className="text-muted text-xs leading-5">
								This can continue in the background; progress resumes if you
								refresh the page.
							</span>
						</div>
					)}
					{sessionMessage && (
						<Notice className="mt-2" tone={"success"} announce="status">
							{sessionMessage}
						</Notice>
					)}
					{(sessionError || actionError) && (
						<Notice className="mt-2" tone={"error"} announce="alert">
							{sessionError ?? actionError}
						</Notice>
					)}

					<TabPanel id="mix" shouldForceMount className="data-[inert]:hidden">
						<div className="rounded-md border border-ui-border bg-surface text-fg shadow-sm p-4 mt-3">
							<div className="flex flex-col justify-between items-start min-[600px]:items-center min-[600px]:flex-row gap-2">
								<div>
									<h6 className="leading-6 font-semibold tracking-tight font-medium tracking-[0.001em] leading-[1.6] text-xl">
										Channel mix
									</h6>
									<p className="leading-6 text-muted text-sm">
										{channelMixScope === "all_recordings"
											? "All recordings while the bot was continuously connected to this channel are shown on one timeline."
											: "Only recordings overlapping this selected session are shown on one timeline (anchor-style)."}
									</p>
								</div>
								<TooltipTrigger delay={400}>
									<Button
										variant="primary"
										isDisabled={!mixCanGenerate || generateMixState.isLoading}
										onPress={() => void createChannelMix()}
									>
										{generateMixState.isLoading
											? "Starting…"
											: mixRenderDirty || channelMix?.status === "ready"
												? "Regenerate channel mix"
												: channelMix?.status === "failed"
													? "Retry mix"
													: "Generate channel mix"}
									</Button>
									<Tooltip>
										{channelMix?.reason?.message ??
											(channelMix?.can_generate === false
												? "Every recording in this mix must be finalized"
												: undefined)}
									</Tooltip>
								</TooltipTrigger>
							</div>

							{channelMixError && !channelMix && (
								<Notice className="mt-3" tone={"error"} announce="alert">
									Channel mix status is unavailable.{" "}
									<Button
										variant="primary"
										size="sm"
										onPress={() => void refetchChannelMix()}
									>
										Retry status
									</Button>
								</Notice>
							)}
							{channelMix && (
								<>
									{channelMix.reason && channelMix.status !== "ready" && (
										<Notice
											className="mt-3"
											tone={channelMix.status === "failed" ? "error" : "info"}
											announce={
												(channelMix.status === "failed" ? "error" : "info") ===
												"error"
													? "alert"
													: "status"
											}
										>
											{channelMix.reason.message}
										</Notice>
									)}
									{mixProcessing && (
										<ChannelMixProgress progress={channelMix.progress} />
									)}
									{channelMix.status === "idle" && (
										<p className="leading-6 text-muted text-sm mt-2">
											{channelMix.source_count} source recordings found. The mix
											is ready to generate.
										</p>
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
												mixCanGenerate
													? () => void createChannelMix()
													: undefined
											}
										/>
									)}
								</>
							)}
							{channelMixActionError && (
								<Notice className="mt-2" tone={"error"} announce="alert">
									{channelMixActionError}
								</Notice>
							)}
						</div>
					</TabPanel>
				</Tabs>
			</PlaybackActionsPanel>

			<SessionClipEditorPanel panelRef={clipEditorRef}>
				<div className="flex items-center flex-wrap flex-row gap-2 mb-3">
					<p className="leading-6">
						{playbackTab === "silence" ? "Silence-free selected" : "Selected"}{" "}
						range: {formatDuration(selection[0] / 1_000)} –{" "}
						{formatDuration(selection[1] / 1_000)}
					</p>
					{clipSelectionIsValid && (
						<Badge tone={"success"} size={"sm"}>
							Valid clip duration
						</Badge>
					)}
					{playbackTab === "normal" && stampClipRequested && (
						<Badge tone={"info"} size={"sm"}>
							Drafted from stamp
						</Badge>
					)}
				</div>
				{playbackTab === "normal" && stampClipRequested && (
					<span className="text-muted text-xs leading-5">
						Fine seek: Arrow 0.1s · Shift+Arrow 1s · Ctrl/⌘+Arrow 5s · I/O set
						the left/right edges · E sets the nearest edge.
					</span>
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
						step={100}
						value={selection}
						minValue={0}
						maxValue={Math.max(1, activeDurationMs)}
						onChange={(value) => {
							if (Array.isArray(value)) changeSelection([value[0], value[1]]);
						}}
					/>
				)}
				<div className="flex items-end flex-nowrap flex-row gap-2 min-w-0">
					<TextField
						label="Clip name"
						value={clipName}
						className="flex-1 min-w-0"
						onChange={(value) => setClipName(value)}
						inputClassName="h-10 shrink-0"
					/>
					<Button
						className="h-10 flex-none"
						variant="primary"
						isDisabled={clipState.isLoading}
						onPress={() => void createSelectedClip()}
					>
						Create clip
					</Button>
				</div>
				{clipMessage && (
					<Notice className="mt-4" tone={"success"} announce="status">
						{clipMessage}
					</Notice>
				)}
				{clipError && (
					<Notice className="mt-4" tone={"error"} announce="alert">
						{clipError}
					</Notice>
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
				<div className="rounded-md border border-ui-border bg-surface text-fg shadow-sm p-4">
					<h6 className="font-medium tracking-[0.001em] text-xl">
						Channel journey
					</h6>
					<p className="leading-6">{manifest.channel_journey.join(" → ")}</p>
				</div>
			)}
		</div>
	);
}
