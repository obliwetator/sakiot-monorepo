import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useLocation } from "react-router-dom";
import { useGetAuthDetailsQuery } from "../../app/apiSlice";
import { isGuildAdmin } from "../../shared/permissions";
import {
	Badge,
	Notice,
	Slider,
	Tab,
	TabList,
	TabPanel,
	Tabs,
} from "../../shared/ui";
import { formatDuration } from "../../utils/formatTime";
import { AudioEventTimeline } from "./AudioEventTimeline";
import { ChannelMixPanel } from "./ChannelMixPanel";
import { ClipRangeEditor } from "./ClipRangeEditor";
import { useChannelMixPreferences } from "./channelMixPreferences";
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
import { visiblePlaybackTab } from "./playbackTabs";
import {
	type RecordingDeletionJob,
	RecordingDeletionStatus,
	RemoveRecordingControl,
} from "./RecordingDeletion";
import { SessionClipForm } from "./SessionClipForm";
import { SessionMediaActions } from "./SessionMediaActions";
import { SessionPlaybackTimeline } from "./SessionPlaybackTimeline";
import { SessionWaveform } from "./SessionWaveform";
import { SilenceFreePlayer } from "./SilenceFreePlayer";
import { useSegmentedSessionPlayback } from "./useSegmentedSessionPlayback";
import { useSessionChannelMix } from "./useSessionChannelMix";
import { useSessionManifest } from "./useSessionManifest";
import { useSessionSelectionController } from "./useSessionSelectionController";
import { useSilenceFreePlayback } from "./useSilenceFreePlayback";
import { useSilenceRemoval } from "./useSilenceRemoval";

export function LogicalSessionPlayer(props: { sessionId: string }) {
	const channelMixPreferences = useChannelMixPreferences();
	const channelMixScope = channelMixPreferences.options.scope;
	const location = useLocation();
	const { data: authDetails } = useGetAuthDetailsQuery();
	const [deletion, setDeletion] = useState<RecordingDeletionJob | null>(null);
	const deepLink = useMemo(
		() => parseSessionDeepLink(location.search),
		[location.search],
	);
	const deepLinkedPositionMs = deepLink?.positionMs ?? null;
	const stampClipRequested = deepLink?.fromStamp === true;
	const {
		data: manifest,
		isLoading,
		isError,
	} = useSessionManifest(props.sessionId);

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
			stopMix();
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
	// At most one of the three players may be audible: starting one stops the
	// others.
	function stopMix() {
		mixStopRef.current();
	}
	function stopSessionPlayback() {
		normal.stop();
		silence.stop();
	}
	const channelMix = useSessionChannelMix({
		sessionId: props.sessionId,
		scope: channelMixScope,
		manifestState: manifest?.state ?? null,
		stopPlayback: () => {
			stopMix();
			stopSessionPlayback();
		},
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
	const showChannelMixTab = Boolean(channelMix.mix?.tracks.length);
	const [mixChosen, setMixChosen] = useState(false);
	const activePlaybackTab = visiblePlaybackTab(
		mixChosen,
		showChannelMixTab,
		playbackTab,
	);
	useEffect(() => {
		// The mix lost its tracks while chosen: stop it and forget the choice, so
		// a later mix does not reopen its tab unasked.
		if (mixChosen && !showChannelMixTab) {
			mixStopRef.current();
			setMixChosen(false);
		}
	}, [mixChosen, showChannelMixTab]);
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
	const seek = (position: number) =>
		normal.seek(position, selection, loopSelection);
	const seekSilence = (position: number) =>
		silence.seek(position, selection, loopSelection);
	const togglePlay = () => {
		rememberActivePlayback("normal");
		stopMix();
		normal.togglePlay(selection, loopSelection);
	};
	const toggleSilencePlay = () => {
		rememberActivePlayback("silence");
		stopMix();
		silence.togglePlay(selection, loopSelection);
	};
	const toggleActivePreviewWithMix = () => {
		stopMix();
		toggleActivePreview();
	};
	const selectPlaybackTabFromTabs = (tab: "normal" | "silence" | "mix") => {
		if (tab === "mix") {
			stopSessionPlayback();
			setMixChosen(true);
			return;
		}
		setMixChosen(false);
		stopMix();
		selectPlaybackTab(tab);
	};
	const selectPlaybackChannel = (channelId: string | null) => {
		const next = isolateSessionChannel(normalizedSegments, channelId);
		normal.restartWithSegments(next);
		setSelectedChannelId(channelId);
		normal.setSeekPreviewMs(null);
	};

	if (deletion) return <RecordingDeletionStatus job={deletion} />;
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
	const canDeleteRecording = isGuildAdmin(
		authDetails?.guilds?.find((guild) => guild.id === manifest.guild_id) ??
			null,
	);

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
			{canDeleteRecording && manifest.state === "finalized" && (
				<RemoveRecordingControl
					guildId={manifest.guild_id}
					sessionId={props.sessionId}
					onRemoved={setDeletion}
				/>
			)}

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

					<SessionMediaActions
						removal={removal}
						finalized={manifest.state === "finalized"}
						playbackError={actionError}
					/>

					<TabPanel id="mix" shouldForceMount className="data-[inert]:hidden">
						<ChannelMixPanel
							sessionId={props.sessionId}
							channelMix={channelMix}
							preferences={channelMixPreferences}
							volume={volume}
							playbackRate={playbackRate}
							onVolumeChange={setVolume}
							onPlaybackRateChange={setPlaybackRate}
							onBeforePlay={stopSessionPlayback}
							onRegisterStop={registerMixStop}
							onPlaybackUse={rememberLastPlayback}
							onPlaybackClear={clearLastPlayback}
						/>
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
				<SessionClipForm
					sessionId={props.sessionId}
					selection={selection}
					silenceFree={playbackTab === "silence"}
				/>
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
