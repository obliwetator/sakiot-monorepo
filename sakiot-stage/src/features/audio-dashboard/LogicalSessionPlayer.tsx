import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import PauseIcon from "@mui/icons-material/Pause";
import PlayArrowIcon from "@mui/icons-material/PlayArrow";
import Accordion from "@mui/material/Accordion";
import AccordionDetails from "@mui/material/AccordionDetails";
import AccordionSummary from "@mui/material/AccordionSummary";
import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import FormControl from "@mui/material/FormControl";
import InputLabel from "@mui/material/InputLabel";
import MenuItem from "@mui/material/MenuItem";
import Paper from "@mui/material/Paper";
import Select from "@mui/material/Select";
import Slider from "@mui/material/Slider";
import Stack from "@mui/material/Stack";
import TextField from "@mui/material/TextField";
import Typography from "@mui/material/Typography";
import type Hls from "hls.js";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useLocation } from "react-router-dom";
import {
	BASE_API_URL,
	useCreateSessionClipMutation,
	useGetSessionManifestQuery,
} from "../../app/apiSlice";
import { authedFetch } from "../../app/authedFetch";
import { formatDuration } from "../../utils/formatTime";
import { AudioEventTimeline } from "./AudioEventTimeline";
import { ClipRangeEditor } from "./ClipRangeEditor";
import { applyEdge, type SelectionEdge } from "./clipSelection";
import {
	isValidClipSelection,
	reconcileSessionSelection,
	type SelectionManifest,
	type SessionSelection,
} from "./logicalSessionSelection";
import {
	isolateSessionChannel,
	normalizeSessionSegments,
	type PlaybackSegment,
} from "./logicalSessionTimeline";
import { SessionWaveform } from "./SessionWaveform";
import { TimelineRow } from "./timelineLayout";

const ARROW_SEEK_MS = 5_000;
const CTRL_ARROW_SEEK_MS = 30_000;

function absoluteMediaUrl(path: string): string {
	return new URL(
		path,
		new URL(BASE_API_URL, window.location.origin),
	).toString();
}

function saveBlob(blob: Blob, fileName: string) {
	const url = URL.createObjectURL(blob);
	const anchor = document.createElement("a");
	anchor.href = url;
	anchor.download = fileName;
	anchor.click();
	URL.revokeObjectURL(url);
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

function isSameMediaSegment(
	left: PlaybackSegment | null,
	right: PlaybackSegment | undefined,
): boolean {
	if (!left || !right || left.kind === "silence" || right.kind === "silence") {
		return false;
	}
	if (left.kind !== right.kind || left.start_ms !== right.start_ms)
		return false;
	if (left.audio_file_id && right.audio_file_id) {
		return left.audio_file_id === right.audio_file_id;
	}
	return Boolean(left.media_url && left.media_url === right.media_url);
}

export function LogicalSessionPlayer(props: { sessionId: string }) {
	const location = useLocation();
	const deepLinkedPositionMs = useMemo(() => {
		const rawPosition = new URLSearchParams(location.search).get("t");
		if (rawPosition === null) return null;
		const seconds = Number(rawPosition);
		return Number.isFinite(seconds) && seconds >= 0 ? seconds * 1_000 : null;
	}, [location.search]);
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
	const [positionMs, setPositionMs] = useState(0);
	const [seekPreviewMs, setSeekPreviewMs] = useState<number | null>(null);
	const [playing, setPlaying] = useState(false);
	const [selection, setSelection] = useState<[number, number]>([0, 0]);
	const [volume, setVolume] = useState(1);
	const [playbackRate, setPlaybackRate] = useState(1);
	const [actionMessage, setActionMessage] = useState<string | null>(null);
	const [actionError, setActionError] = useState<string | null>(null);
	const [clipName, setClipName] = useState("");
	const [previewing, setPreviewing] = useState(false);
	const [loopSelection, setLoopSelection] = useState(false);
	const [createClip, clipState] = useCreateSessionClipMutation();

	const audioRef = useRef<HTMLAudioElement | null>(null);
	const hlsRef = useRef<Hls | null>(null);
	const animationRef = useRef<number | null>(null);
	const generationRef = useRef(0);
	const positionRef = useRef(0);
	const playingRef = useRef(false);
	const activeSegmentRef = useRef<PlaybackSegment | null>(null);
	const durationRef = useRef(0);
	const segmentsRef = useRef<PlaybackSegment[]>([]);
	const rateRef = useRef(1);
	const volumeRef = useRef(1);
	const appliedDeepLinkRef = useRef<string | null>(null);
	const selectionManifestRef = useRef<SelectionManifest | null>(null);
	const selectionRef = useRef<SessionSelection>([0, 0]);
	/** Where a selection preview stops, and where it resumes when looping. */
	const playbackBoundRef = useRef<{
		stopMs: number;
		loopToMs: number | null;
	} | null>(null);
	const startAtRef = useRef<(position: number, autoplay: boolean) => void>(
		() => {},
	);

	useEffect(() => {
		if (manifest?.state === "finalized") {
			setFinalizedSessionId(props.sessionId);
		}
	}, [manifest?.state, props.sessionId]);

	useEffect(() => {
		positionRef.current = positionMs;
	}, [positionMs]);
	useEffect(() => {
		playingRef.current = playing;
	}, [playing]);
	useEffect(() => {
		selectionRef.current = selection;
	}, [selection]);
	useEffect(() => {
		segmentsRef.current = segments;
	}, [segments]);
	useEffect(() => {
		rateRef.current = playbackRate;
		if (audioRef.current) audioRef.current.playbackRate = playbackRate;
	}, [playbackRate]);
	useEffect(() => {
		volumeRef.current = volume;
		if (audioRef.current) audioRef.current.volume = volume;
	}, [volume]);

	useEffect(() => {
		if (!manifest) return;
		durationRef.current = manifest.duration_ms;
		const nextSelectionManifest = {
			recordingSessionId: manifest.recording_session_id,
			durationMs: manifest.duration_ms,
		};
		const previousSelectionManifest = selectionManifestRef.current;
		selectionManifestRef.current = nextSelectionManifest;
		setSelection((current) =>
			reconcileSessionSelection(
				current,
				previousSelectionManifest,
				nextSelectionManifest,
			),
		);
		setPositionMs((current) => Math.min(current, manifest.duration_ms));
		setSeekPreviewMs((current) =>
			current === null ? null : Math.min(current, manifest.duration_ms),
		);
	}, [manifest]);

	const stopSource = useCallback(() => {
		if (animationRef.current !== null) {
			cancelAnimationFrame(animationRef.current);
			animationRef.current = null;
		}
		hlsRef.current?.destroy();
		hlsRef.current = null;
		if (audioRef.current) {
			audioRef.current.pause();
			audioRef.current.removeAttribute("src");
			audioRef.current.load();
			audioRef.current = null;
		}
	}, []);

	const clearPlaybackBound = useCallback(() => {
		playbackBoundRef.current = null;
		setPreviewing(false);
	}, []);

	/**
	 * Stops or restarts a selection preview. Reported so the callers can leave
	 * playback alone once the bound has taken over. Checked wherever playback
	 * advances, so it holds across a segment boundary.
	 */
	const applyPlaybackBound = useCallback((atMs: number): boolean => {
		const bound = playbackBoundRef.current;
		if (!bound || atMs < bound.stopMs) return false;
		if (bound.loopToMs !== null) {
			startAtRef.current(bound.loopToMs, true);
			return true;
		}
		playbackBoundRef.current = null;
		setPreviewing(false);
		startAtRef.current(bound.stopMs, false);
		return true;
	}, []);

	const startAt = (requestedPosition: number, autoplay: boolean) => {
		generationRef.current += 1;
		const generation = generationRef.current;
		stopSource();
		const durationMs = durationRef.current;
		const position = Math.max(0, Math.min(requestedPosition, durationMs));
		positionRef.current = position;
		setPositionMs(position);

		if (!autoplay || position >= durationMs) {
			playingRef.current = false;
			setPlaying(false);
			return;
		}
		const segment = segmentsRef.current.find(
			(candidate) =>
				position >= candidate.start_ms && position < candidate.end_ms,
		);
		if (!segment) {
			activeSegmentRef.current = null;
			playingRef.current = false;
			setPlaying(false);
			return;
		}
		activeSegmentRef.current = segment;
		playingRef.current = true;
		setPlaying(true);

		const segmentLimit = Math.min(segment.end_ms, durationMs);
		if (segment.kind === "silence") {
			const wallStart = performance.now();
			const logicalStart = position;
			const tick = (wallNow: number) => {
				if (generationRef.current !== generation || !playingRef.current) return;
				const next = logicalStart + (wallNow - wallStart) * rateRef.current;
				if (applyPlaybackBound(next)) return;
				if (next >= segmentLimit) {
					startAtRef.current(segmentLimit, segmentLimit < durationMs);
					return;
				}
				positionRef.current = next;
				setPositionMs(next);
				animationRef.current = requestAnimationFrame(tick);
			};
			animationRef.current = requestAnimationFrame(tick);
			return;
		}

		const mediaUrl = segment.media_url;
		if (!mediaUrl) {
			startAtRef.current(segmentLimit, segmentLimit < durationMs);
			return;
		}
		const audio = new Audio();
		audio.crossOrigin = "use-credentials";
		audio.preload = "auto";
		audio.volume = volumeRef.current;
		audio.playbackRate = rateRef.current;
		audioRef.current = audio;

		const begin = () => {
			if (generationRef.current !== generation) return;
			const localSeconds = Math.max(0, (position - segment.start_ms) / 1_000);
			if (Number.isFinite(audio.duration)) {
				audio.currentTime = Math.min(
					localSeconds,
					Math.max(0, audio.duration - 0.01),
				);
			} else {
				audio.currentTime = localSeconds;
			}
			void audio.play().catch((error: unknown) => {
				if (generationRef.current !== generation) return;
				if (error instanceof DOMException && error.name === "AbortError")
					return;
				setActionError("Browser blocked or failed audio playback.");
				playingRef.current = false;
				setPlaying(false);
			});
		};
		audio.addEventListener("timeupdate", () => {
			if (generationRef.current !== generation) return;
			const logical = segment.start_ms + audio.currentTime * 1_000;
			if (applyPlaybackBound(logical)) return;
			if (logical >= segmentLimit - 20) {
				startAtRef.current(segmentLimit, segmentLimit < durationMs);
				return;
			}
			positionRef.current = logical;
			setPositionMs(logical);
		});
		audio.addEventListener("ended", () => {
			if (generationRef.current === generation) {
				startAtRef.current(segmentLimit, segmentLimit < durationMs);
			}
		});
		audio.addEventListener("error", () => {
			if (generationRef.current === generation) {
				setActionError(
					`Could not load segment ${segment.segment_index ?? ""}.`,
				);
				playingRef.current = false;
				setPlaying(false);
			}
		});

		if (segment.kind === "active_hls" && segment.hls_playlist_url) {
			const hlsUrl = absoluteMediaUrl(segment.hls_playlist_url);
			if (audio.canPlayType("application/vnd.apple.mpegurl") === "probably") {
				audio.src = hlsUrl;
				audio.addEventListener("loadedmetadata", begin, { once: true });
			} else {
				void import("hls.js").then(({ default: HlsClass }) => {
					if (generationRef.current !== generation) return;
					if (!HlsClass.isSupported()) {
						audio.src = absoluteMediaUrl(mediaUrl);
						audio.addEventListener("loadedmetadata", begin, { once: true });
						return;
					}
					const hls = new HlsClass({
						xhrSetup: (request) => {
							request.withCredentials = true;
						},
						liveSyncDuration: 2,
						liveMaxLatencyDuration: Number.MAX_SAFE_INTEGER,
					});
					hlsRef.current = hls;
					hls.on(HlsClass.Events.MANIFEST_PARSED, begin);
					hls.loadSource(hlsUrl);
					hls.attachMedia(audio);
				});
			}
		} else {
			audio.src = absoluteMediaUrl(mediaUrl);
			audio.addEventListener("loadedmetadata", begin, { once: true });
		}
	};
	startAtRef.current = startAt;

	useEffect(() => {
		if (!manifest || deepLinkedPositionMs === null) return;
		const deepLinkKey = `${props.sessionId}:${deepLinkedPositionMs}`;
		if (appliedDeepLinkRef.current === deepLinkKey) return;
		appliedDeepLinkRef.current = deepLinkKey;
		setSeekPreviewMs(null);
		startAtRef.current(deepLinkedPositionMs, false);
	}, [deepLinkedPositionMs, manifest, props.sessionId]);

	useEffect(
		() => () => {
			generationRef.current += 1;
			stopSource();
		},
		[stopSource],
	);

	const seekWithinBound = useCallback((nextPositionMs: number) => {
		const durationMs = durationRef.current;
		const position = Math.max(0, Math.min(nextPositionMs, durationMs));
		const targetSegment = segmentsRef.current.find(
			(candidate) =>
				position >= candidate.start_ms && position < candidate.end_ms,
		);
		const audio = audioRef.current;
		if (
			playingRef.current &&
			audio &&
			isSameMediaSegment(activeSegmentRef.current, targetSegment)
		) {
			const localSeconds = Math.max(
				0,
				(position - (targetSegment?.start_ms ?? 0)) / 1_000,
			);
			try {
				audio.currentTime = Number.isFinite(audio.duration)
					? Math.min(localSeconds, Math.max(0, audio.duration - 0.01))
					: localSeconds;
				positionRef.current = position;
				setPositionMs(position);
				return;
			} catch {
				// Source replacement below handles media that cannot seek in place.
			}
		}
		startAtRef.current(nextPositionMs, playingRef.current);
	}, []);

	const seek = useCallback(
		(nextPositionMs: number) => {
			// Going somewhere by hand ends the preview rather than letting the bound
			// yank playback back at the next check.
			clearPlaybackBound();
			seekWithinBound(nextPositionMs);
		},
		[clearPlaybackBound, seekWithinBound],
	);

	const selectPlaybackChannel = useCallback(
		(channelId: string | null) => {
			const nextSegments = isolateSessionChannel(normalizedSegments, channelId);
			segmentsRef.current = nextSegments;
			setSelectedChannelId(channelId);
			setSeekPreviewMs(null);
			startAtRef.current(positionRef.current, playingRef.current);
		},
		[normalizedSegments],
	);

	const togglePlay = useCallback(() => {
		clearPlaybackBound();
		if (playingRef.current) {
			generationRef.current += 1;
			stopSource();
			playingRef.current = false;
			setPlaying(false);
			return;
		}
		setActionError(null);
		const start =
			positionRef.current >= durationRef.current ? 0 : positionRef.current;
		startAtRef.current(start, true);
	}, [clearPlaybackBound, stopSource]);

	const setSelectionEdgeFromPlayhead = useCallback((edge: SelectionEdge) => {
		setSeekPreviewMs(null);
		setSelection((current) =>
			applyEdge(current, edge, positionRef.current, durationRef.current),
		);
	}, []);

	const togglePreview = useCallback(() => {
		if (playbackBoundRef.current) {
			clearPlaybackBound();
			generationRef.current += 1;
			stopSource();
			playingRef.current = false;
			setPlaying(false);
			return;
		}
		if (selectionRef.current[1] <= selectionRef.current[0]) return;
		setActionError(null);
		playbackBoundRef.current = {
			stopMs: selectionRef.current[1],
			loopToMs: loopSelection ? selectionRef.current[0] : null,
		};
		setPreviewing(true);
		setSeekPreviewMs(null);
		startAtRef.current(selectionRef.current[0], true);
	}, [clearPlaybackBound, loopSelection, stopSource]);

	// A loop toggled mid-preview takes effect on the current pass.
	useEffect(() => {
		const bound = playbackBoundRef.current;
		if (!bound) return;
		playbackBoundRef.current = {
			stopMs: bound.stopMs,
			loopToMs: loopSelection ? selectionRef.current[0] : null,
		};
	}, [loopSelection]);

	useEffect(() => {
		const handlePlaybackShortcut = (event: KeyboardEvent) => {
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

			if (event.key === "i" || event.key === "I") {
				event.preventDefault();
				setSelectionEdgeFromPlayhead("start");
				return;
			}

			if (event.key === "o" || event.key === "O") {
				event.preventDefault();
				setSelectionEdgeFromPlayhead("end");
				return;
			}

			if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
			event.preventDefault();
			const distance = event.ctrlKey ? CTRL_ARROW_SEEK_MS : ARROW_SEEK_MS;
			const direction = event.key === "ArrowRight" ? 1 : -1;
			setSeekPreviewMs(null);
			seek(positionRef.current + direction * distance);
		};

		window.addEventListener("keydown", handlePlaybackShortcut);
		return () => window.removeEventListener("keydown", handlePlaybackShortcut);
	}, [seek, setSelectionEdgeFromPlayhead, togglePlay]);

	const downloadRange = async (removeSilence: boolean) => {
		setActionError(null);
		setActionMessage(null);
		const query = new URLSearchParams({
			start: String(selection[0] / 1_000),
			end: String(selection[1] / 1_000),
		});
		if (removeSilence) query.set("remove_silence", "true");
		const response = await authedFetch(
			`audio/sessions/${props.sessionId}/download?${query}`,
		);
		if (!response.ok) {
			setActionError(`Composition failed (${response.status}).`);
			return;
		}
		saveBlob(
			await response.blob(),
			`session-${props.sessionId}${removeSilence ? "-silence-free" : ""}.ogg`,
		);
	};

	const removeSilenceRange = async () => {
		setActionError(null);
		const response = await authedFetch(
			`audio/sessions/${props.sessionId}/remove-silence`,
			{
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({
					start: selection[0] / 1_000,
					end: selection[1] / 1_000,
				}),
			},
		);
		if (!response.ok) {
			setActionError(`Silence removal failed (${response.status}).`);
			return;
		}
		saveBlob(
			await response.blob(),
			`session-${props.sessionId}-silence-free.ogg`,
		);
	};

	const createSelectedClip = async () => {
		setActionError(null);
		setActionMessage(null);
		try {
			const response = await createClip({
				recording_session_id: props.sessionId,
				start: selection[0] / 1_000,
				end: selection[1] / 1_000,
				name: clipName.trim() || undefined,
			}).unwrap();
			setActionMessage(`Clip created: ${response.name}`);
			setClipName("");
		} catch {
			setActionError("Clip creation failed. Select 1-20 seconds.");
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

	const durationSeconds = manifest.duration_ms / 1_000;
	const displayedPositionMs = seekPreviewMs ?? positionMs;
	const currentSegment = segments.find(
		(segment) =>
			displayedPositionMs >= segment.start_ms &&
			displayedPositionMs < segment.end_ms,
	);
	const hasChannelJourney = new Set(manifest.channel_journey).size > 1;
	const clipSelectionIsValid = isValidClipSelection(selection);

	return (
		<Box sx={{ pb: 4 }}>
			<Stack direction="row" spacing={1} flexWrap="wrap" sx={{ mb: 2 }}>
				<Chip label={`Session ${manifest.recording_session_id}`} />
				<Chip
					label={manifest.state}
					color={manifest.state === "active" ? "error" : "default"}
				/>
				<Chip label={`User ${manifest.user_id}`} />
				<Chip
					label={`${physicalFragments.length} physical ${
						physicalFragments.length === 1 ? "file" : "files"
					}`}
					variant="outlined"
				/>
				{currentSegment && (
					<Chip
						label={
							currentSegment.reason === "channel_filtered"
								? `Channel ${currentSegment.channel_id ?? "?"} muted`
								: currentSegment.kind === "silence"
									? `Silence · ${currentSegment.reason ?? "gap"}`
									: `Channel ${currentSegment.channel_id ?? "?"}`
						}
						color={
							currentSegment.reason === "channel_filtered"
								? "default"
								: currentSegment.kind === "silence"
									? "warning"
									: "primary"
						}
					/>
				)}
			</Stack>
			<Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
				Started {new Date(manifest.started_at_ms).toLocaleString()} · duration{" "}
				{formatDuration(durationSeconds)}
			</Typography>

			<Paper variant="outlined" sx={{ p: 2, my: 2 }}>
				<TimelineRow label="Waveform" labelAlign="flex-start" sx={{ mb: 1 }}>
					<SessionWaveform
						key={props.sessionId}
						sessionId={props.sessionId}
						positionMs={displayedPositionMs}
						durationMs={manifest.duration_ms}
						onSeek={seek}
					/>
				</TimelineRow>

				<TimelineRow label="Position">
					<Slider
						aria-label="Logical playback position"
						min={0}
						max={Math.max(0.001, durationSeconds)}
						step={0.01}
						value={Math.min(durationSeconds, displayedPositionMs / 1_000)}
						onChange={(_event, value) =>
							setSeekPreviewMs(Number(value) * 1_000)
						}
						onChangeCommitted={(_event, value) => {
							const nextPositionMs = Number(value) * 1_000;
							setSeekPreviewMs(null);
							seek(nextPositionMs);
						}}
						valueLabelDisplay="auto"
						valueLabelFormat={formatDuration}
						sx={{
							display: "block",
							py: 1.5,
							"& .MuiSlider-thumb, & .MuiSlider-track": {
								transition: "none",
							},
						}}
					/>
				</TimelineRow>

				<TimelineRow sx={{ mb: 1.5 }}>
					<Stack
						direction={{ xs: "column", sm: "row" }}
						justifyContent="space-between"
						spacing={0.5}
					>
						<Typography
							variant="body2"
							sx={{ fontVariantNumeric: "tabular-nums" }}
						>
							Recording {formatDuration(displayedPositionMs / 1_000)} /{" "}
							{formatDuration(durationSeconds)}
						</Typography>
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
					</Stack>
				</TimelineRow>

				<AudioEventTimeline
					events={manifest.events}
					durationMs={manifest.duration_ms}
					positionMs={displayedPositionMs}
					startedAtMs={manifest.started_at_ms}
					onSeek={seek}
				/>
			</Paper>

			<Stack
				direction={{ xs: "column", md: "row" }}
				spacing={2}
				alignItems="center"
			>
				<Button
					variant="contained"
					onClick={togglePlay}
					startIcon={playing ? <PauseIcon /> : <PlayArrowIcon />}
				>
					{playing ? "Pause" : "Play"}
				</Button>
				<Box sx={{ minWidth: 180, flex: 1 }}>
					<Typography variant="caption">Volume</Typography>
					<Slider
						min={0}
						max={1}
						step={0.05}
						value={volume}
						onChange={(_event, value) => setVolume(Number(value))}
					/>
				</Box>
				<Box sx={{ minWidth: 180, flex: 1 }}>
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

			<Paper sx={{ p: 2, my: 2 }}>
				<Stack
					direction="row"
					spacing={1}
					alignItems="center"
					flexWrap="wrap"
					sx={{ mb: 1.5 }}
				>
					<Typography>
						Selected range: {formatDuration(selection[0] / 1_000)} –{" "}
						{formatDuration(selection[1] / 1_000)}
					</Typography>
					{clipSelectionIsValid && (
						<Chip label="Valid clip duration" color="success" size="small" />
					)}
				</Stack>

				<ClipRangeEditor
					sessionId={props.sessionId}
					durationMs={manifest.duration_ms}
					selection={selection}
					onSelectionChange={setSelection}
					positionMs={displayedPositionMs}
					onSeek={seek}
					onSetEdgeFromPlayhead={setSelectionEdgeFromPlayhead}
					onPreview={togglePreview}
					previewing={previewing}
					loop={loopSelection}
					onLoopChange={setLoopSelection}
				/>

				<Slider
					aria-label="Logical action range"
					min={0}
					max={Math.max(1, manifest.duration_ms)}
					step={100}
					value={selection}
					onChange={(_event, value) => {
						if (Array.isArray(value)) setSelection([value[0], value[1]]);
					}}
					valueLabelDisplay="auto"
					valueLabelFormat={(value) => formatDuration(value / 1_000)}
					disableSwap
				/>
				<Stack
					direction={{ xs: "column", sm: "row" }}
					spacing={1}
					flexWrap="wrap"
				>
					<Button variant="outlined" onClick={() => void downloadRange(false)}>
						Download range
					</Button>
					<Button variant="outlined" onClick={() => void downloadRange(true)}>
						Download without silence
					</Button>
					<Button variant="outlined" onClick={() => void removeSilenceRange()}>
						Remove silence
					</Button>
					<TextField
						size="small"
						label="Clip name"
						value={clipName}
						onChange={(event) => setClipName(event.target.value)}
					/>
					<Button
						variant="contained"
						onClick={() => void createSelectedClip()}
						disabled={clipState.isLoading}
					>
						Create clip
					</Button>
				</Stack>
				{actionMessage && (
					<Alert severity="success" sx={{ mt: 2 }}>
						{actionMessage}
					</Alert>
				)}
				{actionError && (
					<Alert severity="error" sx={{ mt: 2 }}>
						{actionError}
					</Alert>
				)}
			</Paper>

			<Accordion variant="outlined" disableGutters sx={{ my: 2 }}>
				<AccordionSummary
					expandIcon={<ExpandMoreIcon />}
					aria-controls={`session-${props.sessionId}-physical-content`}
					id={`session-${props.sessionId}-physical-header`}
				>
					<Stack
						direction="row"
						spacing={1}
						alignItems="center"
						flexWrap="wrap"
						useFlexGap
					>
						<Typography>Physical recordings</Typography>
						<Chip
							size="small"
							variant="outlined"
							label={`${displayedFragments.length} ${
								displayedFragments.length === 1 ? "file" : "files"
							}`}
						/>
						{effectiveChannelId && (
							<Chip
								size="small"
								color="primary"
								label={`Channel ${effectiveChannelId} only`}
							/>
						)}
					</Stack>
				</AccordionSummary>
				<AccordionDetails id={`session-${props.sessionId}-physical-content`}>
					<Stack
						direction={{ xs: "column", md: "row" }}
						spacing={2}
						justifyContent="space-between"
						alignItems={{ xs: "stretch", md: "flex-start" }}
					>
						<Typography variant="body2" color="text.secondary">
							Session combines channel-bound files into one timestamp-aligned
							timeline.
						</Typography>
						{channelIds.length > 1 && (
							<FormControl size="small" sx={{ minWidth: 260 }}>
								<InputLabel id={`session-${props.sessionId}-channel-label`}>
									Playback channel
								</InputLabel>
								<Select
									labelId={`session-${props.sessionId}-channel-label`}
									label="Playback channel"
									value={effectiveChannelId ?? ""}
									onChange={(event) =>
										selectPlaybackChannel(event.target.value || null)
									}
								>
									<MenuItem value="">All channels</MenuItem>
									{channelIds.map((channelId) => {
										const count = physicalFragments.filter(
											(fragment) => fragment.channel_id === channelId,
										).length;
										return (
											<MenuItem key={channelId} value={channelId}>
												Channel {channelId} · {count}{" "}
												{count === 1 ? "file" : "files"}
											</MenuItem>
										);
									})}
								</Select>
							</FormControl>
						)}
					</Stack>
					<Stack spacing={0.75} sx={{ mt: 1.5 }}>
						{displayedFragments.map((fragment, index) => (
							<Button
								key={
									fragment.audio_file_id ??
									`${fragment.start_ms}-${fragment.end_ms}`
								}
								variant="text"
								onClick={() => seek(fragment.start_ms)}
								sx={{
									justifyContent: "flex-start",
									textAlign: "left",
									textTransform: "none",
									px: 1,
								}}
							>
								<Box>
									<Typography variant="body2">
										Fragment {(fragment.segment_index ?? index) + 1} · Channel{" "}
										{fragment.channel_id ?? "?"} ·{" "}
										{formatDuration(fragment.start_ms / 1_000)} –{" "}
										{formatDuration(fragment.end_ms / 1_000)}
									</Typography>
									<Typography variant="caption" color="text.secondary">
										{fragment.file_name ?? `File ${fragment.audio_file_id}`}
									</Typography>
								</Box>
							</Button>
						))}
					</Stack>
					{effectiveChannelId && (
						<Alert severity="info" sx={{ mt: 1.5 }}>
							Only Channel {effectiveChannelId} plays. Other channels stay muted
							while timeline offsets remain unchanged. Downloads and clips still
							use full session.
						</Alert>
					)}
				</AccordionDetails>
			</Accordion>

			{hasChannelJourney && (
				<Paper sx={{ p: 2 }}>
					<Typography variant="h6">Channel journey</Typography>
					<Typography>{manifest.channel_journey.join(" → ")}</Typography>
				</Paper>
			)}
		</Box>
	);
}
