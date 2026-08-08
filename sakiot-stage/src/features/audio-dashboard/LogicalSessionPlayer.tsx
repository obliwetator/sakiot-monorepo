import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import Accordion from "@mui/material/Accordion";
import AccordionDetails from "@mui/material/AccordionDetails";
import AccordionSummary from "@mui/material/AccordionSummary";
import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import FormControl from "@mui/material/FormControl";
import InputLabel from "@mui/material/InputLabel";
import LinearProgress from "@mui/material/LinearProgress";
import MenuItem from "@mui/material/MenuItem";
import Paper from "@mui/material/Paper";
import Select from "@mui/material/Select";
import Slider from "@mui/material/Slider";
import Stack from "@mui/material/Stack";
import Tab from "@mui/material/Tab";
import Tabs from "@mui/material/Tabs";
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
import {
	authedFetch,
	refreshForMediaRetry,
	SESSION_EXPIRED_MESSAGE,
} from "../../app/authedFetch";
import { formatDuration } from "../../utils/formatTime";
import { AudioEventTimeline } from "./AudioEventTimeline";
import { ClipRangeEditor } from "./ClipRangeEditor";
import {
	applyEdge,
	canSetSelectionEdge,
	nearestSelectionEdge,
	type SelectionEdge,
} from "./clipSelection";
import { isResetClipSelectionShortcut } from "./clipSelectionShortcuts";
import {
	isValidClipSelection,
	reconcileSessionSelection,
	resetClipSelection,
	type SelectionManifest,
	type SessionSelection,
	selectionAroundStamp,
	selectionContainsPosition,
} from "./logicalSessionSelection";
import {
	isolateSessionChannel,
	normalizeSessionSegments,
	type PlaybackSegment,
} from "./logicalSessionTimeline";
import { PlaybackControls } from "./PlaybackControls";
import {
	playbackShortcutTargetAcceptsText,
	playbackShortcutTargetOwnsArrows,
} from "./playbackShortcuts";
import { SessionPlaybackTimeline } from "./SessionPlaybackTimeline";
import { SessionWaveform } from "./SessionWaveform";
import { SilenceFreePlayer } from "./SilenceFreePlayer";

const ARROW_SEEK_MS = 5_000;
const CTRL_ARROW_SEEK_MS = 30_000;
const CLIP_ARROW_SEEK_MS = 100;
const CLIP_SHIFT_ARROW_SEEK_MS = 1_000;
const CLIP_CTRL_ARROW_SEEK_MS = 5_000;

type SilenceRemovalStatus = {
	status: "idle" | "processing" | "ready" | "failed";
	progress: number;
};

function parseSilenceRemovalStatus(value: unknown): SilenceRemovalStatus {
	if (!value || typeof value !== "object") {
		return { status: "failed", progress: 0 };
	}
	const candidate = value as { status?: unknown; progress?: unknown };
	const status =
		candidate.status === "processing" ||
		candidate.status === "ready" ||
		candidate.status === "failed"
			? candidate.status
			: "idle";
	const progress =
		typeof candidate.progress === "number" &&
		Number.isFinite(candidate.progress)
			? Math.round(Math.min(100, Math.max(0, candidate.progress)))
			: 0;
	return { status, progress };
}

function absoluteMediaUrl(path: string): string {
	return new URL(
		path,
		new URL(BASE_API_URL, window.location.origin),
	).toString();
}

function saveBlob(blob: Blob, fileName: string) {
	const url = URL.createObjectURL(blob);
	try {
		const anchor = document.createElement("a");
		anchor.href = url;
		anchor.download = fileName;
		document.body.appendChild(anchor);
		anchor.click();
		anchor.remove();
	} catch {
		window.open(url, "_blank");
	} finally {
		window.setTimeout(() => URL.revokeObjectURL(url), 1_000);
	}
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
	const deepLink = useMemo(() => {
		const params = new URLSearchParams(location.search);
		const rawPosition = params.get("t");
		if (rawPosition === null) return null;
		const seconds = Number(rawPosition);
		if (!Number.isFinite(seconds) || seconds < 0) return null;
		return {
			positionMs: seconds * 1_000,
			fromStamp: params.get("clip") === "stamp",
			silenceFree: params.get("timeline") === "silence-free",
		};
	}, [location.search]);
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
	const [silencePositionMs, setSilencePositionMs] = useState(0);
	const [silenceSeekPreviewMs, setSilenceSeekPreviewMs] = useState<
		number | null
	>(null);
	const [silenceDurationMs, setSilenceDurationMs] = useState(0);
	const [silencePlaying, setSilencePlaying] = useState(false);
	const [silencePlaybackError, setSilencePlaybackError] = useState<
		string | null
	>(null);
	const [silenceRetryKey, setSilenceRetryKey] = useState(0);
	const silenceMediaRetryRef = useRef(false);
	const [selection, setSelection] = useState<[number, number]>([0, 0]);
	const [volume, setVolume] = useState(1);
	const [playbackRate, setPlaybackRate] = useState(1);
	const [actionError, setActionError] = useState<string | null>(null);
	const [clipMessage, setClipMessage] = useState<string | null>(null);
	const [clipError, setClipError] = useState<string | null>(null);
	const [sessionMessage, setSessionMessage] = useState<string | null>(null);
	const [sessionError, setSessionError] = useState<string | null>(null);
	const [sessionAction, setSessionAction] = useState<
		"download" | "silence" | "silence-download" | null
	>(null);
	const [playbackTab, setPlaybackTab] = useState<"normal" | "silence">(
		"normal",
	);
	const [silenceFreeUrl, setSilenceFreeUrl] = useState<string | null>(null);
	const [silenceRemoval, setSilenceRemoval] = useState<SilenceRemovalStatus>({
		status: "idle",
		progress: 0,
	});
	const [selectionHint, setSelectionHint] = useState<string | null>(null);
	const [clipName, setClipName] = useState("");
	const [previewing, setPreviewing] = useState(false);
	const [loopSelection, setLoopSelection] = useState(false);
	const [createClip, clipState] = useCreateSessionClipMutation();

	const audioRef = useRef<HTMLAudioElement | null>(null);
	const silenceAudioRef = useRef<HTMLAudioElement | null>(null);
	const hlsRef = useRef<Hls | null>(null);
	const animationRef = useRef<number | null>(null);
	const generationRef = useRef(0);
	// One refresh-retry between successful loads: media elements can't report
	// the HTTP status, so a stale access token looks like a plain load error.
	// Reset whenever a segment starts playing; a retry that fails (or an
	// error while a retry is in flight) surfaces the error without looping.
	const mediaRetryRef = useRef(false);
	const positionRef = useRef(0);
	const playingRef = useRef(false);
	const activeSegmentRef = useRef<PlaybackSegment | null>(null);
	const durationRef = useRef(0);
	const silenceDurationRef = useRef(0);
	const silencePositionRef = useRef(0);
	const silencePlayingRef = useRef(false);
	const segmentsRef = useRef<PlaybackSegment[]>([]);
	const rateRef = useRef(1);
	const volumeRef = useRef(1);
	const appliedDeepLinkRef = useRef<string | null>(null);
	const clipEditorRef = useRef<HTMLDivElement | null>(null);
	const selectionManifestRef = useRef<SelectionManifest | null>(null);
	const selectionRef = useRef<SessionSelection>([0, 0]);
	const normalSelectionRef = useRef<SessionSelection>([0, 0]);
	const silenceSelectionRef = useRef<SessionSelection | null>(null);
	const playbackTabRef = useRef<"normal" | "silence">("normal");
	const loopSelectionRef = useRef(false);
	const silenceRemovalRequestedRef = useRef(false);
	const silenceFreeMediaUrl = useMemo(
		() => absoluteMediaUrl(`audio/sessions/${props.sessionId}/silence-free`),
		[props.sessionId],
	);
	/** Where a selection preview stops, and where it resumes when looping. */
	const playbackBoundRef = useRef<{
		stopMs: number;
		loopToMs: number | null;
	} | null>(null);
	const silencePlaybackBoundRef = useRef<{
		stopMs: number;
		loopToMs: number | null;
	} | null>(null);
	const startAtRef = useRef<(position: number, autoplay: boolean) => void>(
		() => {},
	);
	const startSilenceAtRef = useRef<
		(position: number, autoplay: boolean) => void
	>(() => {});

	useEffect(() => {
		if (manifest?.state === "finalized") {
			setFinalizedSessionId(props.sessionId);
		}
	}, [manifest?.state, props.sessionId]);

	useEffect(() => {
		positionRef.current = positionMs;
	}, [positionMs]);
	useEffect(() => {
		silencePositionRef.current = silencePositionMs;
	}, [silencePositionMs]);
	useEffect(() => {
		if (!selectionHint) return;
		const timeout = globalThis.setTimeout(() => setSelectionHint(null), 4_000);
		return () => globalThis.clearTimeout(timeout);
	}, [selectionHint]);
	useEffect(() => {
		playingRef.current = playing;
	}, [playing]);
	useEffect(() => {
		silencePlayingRef.current = silencePlaying;
	}, [silencePlaying]);
	useEffect(() => {
		selectionRef.current = selection;
	}, [selection]);
	useEffect(() => {
		loopSelectionRef.current = loopSelection;
	}, [loopSelection]);
	useEffect(() => {
		segmentsRef.current = segments;
	}, [segments]);
	useEffect(() => {
		rateRef.current = playbackRate;
		if (audioRef.current) audioRef.current.playbackRate = playbackRate;
		if (silenceAudioRef.current) {
			silenceAudioRef.current.playbackRate = playbackRate;
		}
	}, [playbackRate]);
	useEffect(() => {
		volumeRef.current = volume;
		if (audioRef.current) audioRef.current.volume = volume;
		if (silenceAudioRef.current) silenceAudioRef.current.volume = volume;
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
		const nextSelection = reconcileSessionSelection(
			normalSelectionRef.current,
			previousSelectionManifest,
			nextSelectionManifest,
		);
		normalSelectionRef.current = nextSelection;
		if (playbackTabRef.current === "normal") {
			selectionRef.current = nextSelection;
			setSelection(nextSelection);
		}
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

	const stopSilenceSource = useCallback(() => {
		const audio = silenceAudioRef.current;
		if (audio) audio.pause();
		silencePlayingRef.current = false;
		setSilencePlaying(false);
	}, []);

	const clearSilencePlaybackBound = useCallback(() => {
		silencePlaybackBoundRef.current = null;
		setPreviewing(false);
	}, []);

	const disableLoopSelection = useCallback(() => {
		if (!loopSelectionRef.current) return;
		loopSelectionRef.current = false;
		setLoopSelection(false);
	}, []);

	const startSilenceAt = useCallback(
		(requestedPosition: number, autoplay: boolean) => {
			const audio = silenceAudioRef.current;
			const duration = silenceDurationRef.current;
			const position = Math.max(0, Math.min(requestedPosition, duration));
			silencePositionRef.current = position;
			setSilencePositionMs(position);
			setSilenceSeekPreviewMs(null);
			if (!audio) return;
			try {
				audio.currentTime = position / 1_000;
			} catch {
				setSilencePlaybackError("Silence-free audio could not be seeked.");
				return;
			}
			if (!autoplay || position >= duration) {
				audio.pause();
				silencePlayingRef.current = false;
				setSilencePlaying(false);
				return;
			}
			setSilencePlaybackError(null);
			void audio.play().catch(() => {
				silencePlayingRef.current = false;
				setSilencePlaying(false);
				clearSilencePlaybackBound();
				setSilencePlaybackError(
					"Browser blocked or failed silence-free playback.",
				);
			});
		},
		[clearSilencePlaybackBound],
	);
	startSilenceAtRef.current = startSilenceAt;

	const applySilencePlaybackBound = useCallback((atMs: number): boolean => {
		const bound = silencePlaybackBoundRef.current;
		if (!bound || atMs < bound.stopMs) return false;
		if (bound.loopToMs !== null) {
			startSilenceAtRef.current(bound.loopToMs, true);
			return true;
		}
		silencePlaybackBoundRef.current = null;
		setPreviewing(false);
		startSilenceAtRef.current(bound.stopMs, false);
		return true;
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
			clearPlaybackBound();
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
			if (applyPlaybackBound(segmentLimit)) return;
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
			mediaRetryRef.current = false;
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
				clearPlaybackBound();
			});
		};
		const failSegment = (message: string) => {
			setActionError(message);
			playingRef.current = false;
			setPlaying(false);
			clearPlaybackBound();
		};
		// One refresh-retry per successful load: a stale access token makes
		// media loads fail without a readable status, so try refreshing and
		// restarting the segment from the current position once. If the
		// refresh fails, the session is gone — say so instead of showing a
		// generic playback error.
		const retryOrFail = () => {
			if (generationRef.current !== generation) return;
			if (mediaRetryRef.current) return;
			mediaRetryRef.current = true;
			void refreshForMediaRetry().then((ok) => {
				if (generationRef.current !== generation) return;
				if (ok) {
					setActionError(null);
					startAtRef.current(positionRef.current, true);
				} else {
					failSegment(SESSION_EXPIRED_MESSAGE);
				}
			});
		};
		audio.addEventListener("pause", () => {
			if (generationRef.current !== generation) return;
			playingRef.current = false;
			setPlaying(false);
			clearPlaybackBound();
		});
		audio.addEventListener("timeupdate", () => {
			if (generationRef.current !== generation) return;
			const logical = segment.start_ms + audio.currentTime * 1_000;
			if (applyPlaybackBound(logical)) return;
			if (logical >= segmentLimit - 20) {
				if (applyPlaybackBound(segmentLimit)) return;
				startAtRef.current(segmentLimit, segmentLimit < durationMs);
				return;
			}
			positionRef.current = logical;
			setPositionMs(logical);
		});
		audio.addEventListener("ended", () => {
			if (generationRef.current === generation) {
				if (applyPlaybackBound(segmentLimit)) return;
				startAtRef.current(segmentLimit, segmentLimit < durationMs);
			}
		});
		audio.addEventListener("error", () => {
			if (generationRef.current !== generation) return;
			if (mediaRetryRef.current) {
				failSegment(`Could not load segment ${segment.segment_index ?? ""}.`);
				return;
			}
			retryOrFail();
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
					hls.on(HlsClass.Events.ERROR, (_event, data) => {
						if (generationRef.current !== generation) return;
						if (!data.fatal) return;
						if (mediaRetryRef.current) {
							failSegment(
								`Could not load segment ${segment.segment_index ?? ""}.`,
							);
							return;
						}
						retryOrFail();
					});
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

	const updateSilenceDuration = useCallback(
		(audio: HTMLAudioElement) => {
			if (!Number.isFinite(audio.duration) || audio.duration <= 0) return;
			const duration = audio.duration * 1_000;
			const previousDuration = silenceDurationRef.current;
			silenceDurationRef.current = duration;
			setSilenceDurationMs(duration);
			audio.volume = volumeRef.current;
			audio.playbackRate = rateRef.current;

			const current = silenceSelectionRef.current;
			const nextSelection = reconcileSessionSelection(
				current ?? [0, 0],
				current && previousDuration > 0
					? { recordingSessionId: "silence-free", durationMs: previousDuration }
					: null,
				{ recordingSessionId: "silence-free", durationMs: duration },
			);
			silenceSelectionRef.current = nextSelection;
			if (playbackTabRef.current === "silence") {
				selectionRef.current = nextSelection;
				setSelection(nextSelection);
			}

			if (deepLink?.silenceFree && deepLinkedPositionMs !== null) {
				const deepLinkKey = `${props.sessionId}:${deepLinkedPositionMs}:silence-free`;
				if (appliedDeepLinkRef.current !== deepLinkKey) {
					appliedDeepLinkRef.current = deepLinkKey;
					startSilenceAtRef.current(
						Math.min(deepLinkedPositionMs, duration),
						false,
					);
				}
			}
		},
		[deepLink?.silenceFree, deepLinkedPositionMs, props.sessionId],
	);

	useEffect(() => {
		if (!silencePlaying) return;
		let frame: number | null = null;
		const update = () => {
			const audio = silenceAudioRef.current;
			if (!audio || audio.paused) return;
			const nextPosition = audio.currentTime * 1_000;
			if (applySilencePlaybackBound(nextPosition)) return;
			silencePositionRef.current = nextPosition;
			setSilencePositionMs(nextPosition);
			frame = requestAnimationFrame(update);
		};
		frame = requestAnimationFrame(update);
		return () => {
			if (frame !== null) cancelAnimationFrame(frame);
		};
	}, [applySilencePlaybackBound, silencePlaying]);

	useEffect(() => {
		if (!manifest || deepLinkedPositionMs === null || deepLink?.silenceFree)
			return;
		const deepLinkKey = `${props.sessionId}:${deepLinkedPositionMs}:${stampClipRequested}`;
		if (appliedDeepLinkRef.current === deepLinkKey) return;
		appliedDeepLinkRef.current = deepLinkKey;
		const positionMs = Math.min(deepLinkedPositionMs, manifest.duration_ms);
		setSeekPreviewMs(null);
		clearPlaybackBound();
		disableLoopSelection();
		startAtRef.current(positionMs, false);
		if (stampClipRequested) {
			const nextSelection = selectionAroundStamp(
				positionMs,
				manifest.duration_ms,
			);
			normalSelectionRef.current = nextSelection;
			selectionRef.current = nextSelection;
			setSelection(nextSelection);
			requestAnimationFrame(() => {
				clipEditorRef.current?.scrollIntoView({
					behavior: "smooth",
					block: "center",
				});
			});
		}
	}, [
		clearPlaybackBound,
		deepLinkedPositionMs,
		deepLink?.silenceFree,
		disableLoopSelection,
		manifest,
		props.sessionId,
		stampClipRequested,
	]);

	useEffect(
		() => () => {
			generationRef.current += 1;
			stopSource();
			stopSilenceSource();
		},
		[stopSilenceSource, stopSource],
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
			setSelectionHint(null);
			const target = Math.max(0, Math.min(nextPositionMs, durationRef.current));
			const loopRemainsEnabled =
				loopSelectionRef.current &&
				selectionContainsPosition(selectionRef.current, target);
			if (!loopRemainsEnabled) {
				clearPlaybackBound();
				disableLoopSelection();
			} else if (playbackBoundRef.current) {
				playbackBoundRef.current = {
					stopMs: selectionRef.current[1],
					loopToMs: selectionRef.current[0],
				};
			}
			seekWithinBound(target);
		},
		[clearPlaybackBound, disableLoopSelection, seekWithinBound],
	);

	const seekSilence = useCallback(
		(nextPositionMs: number) => {
			setSelectionHint(null);
			setSilenceSeekPreviewMs(null);
			const target = Math.max(
				0,
				Math.min(nextPositionMs, silenceDurationRef.current),
			);
			const loopRemainsEnabled =
				loopSelectionRef.current &&
				selectionContainsPosition(selectionRef.current, target);
			if (!loopRemainsEnabled) {
				clearSilencePlaybackBound();
				disableLoopSelection();
			} else if (silencePlaybackBoundRef.current) {
				silencePlaybackBoundRef.current = {
					stopMs: selectionRef.current[1],
					loopToMs: selectionRef.current[0],
				};
			}
			const audio = silenceAudioRef.current;
			if (audio) {
				try {
					audio.currentTime = target / 1_000;
				} catch {
					setSilencePlaybackError("Silence-free audio could not be seeked.");
					return;
				}
			}
			silencePositionRef.current = target;
			setSilencePositionMs(target);
			if (silencePlaybackBoundRef.current) applySilencePlaybackBound(target);
		},
		[
			applySilencePlaybackBound,
			clearSilencePlaybackBound,
			disableLoopSelection,
		],
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
		if (playingRef.current) {
			clearPlaybackBound();
			generationRef.current += 1;
			stopSource();
			playingRef.current = false;
			setPlaying(false);
			return;
		}
		setActionError(null);
		const activeSelection = selectionRef.current;
		if (loopSelectionRef.current && activeSelection[1] > activeSelection[0]) {
			playbackBoundRef.current = {
				stopMs: activeSelection[1],
				loopToMs: activeSelection[0],
			};
			setPreviewing(true);
			const currentPosition = positionRef.current;
			const start =
				currentPosition >= activeSelection[0] &&
				currentPosition < activeSelection[1]
					? currentPosition
					: activeSelection[0];
			startAtRef.current(start, true);
			return;
		}
		clearPlaybackBound();
		const start =
			positionRef.current >= durationRef.current ? 0 : positionRef.current;
		startAtRef.current(start, true);
	}, [clearPlaybackBound, stopSource]);

	const toggleSilencePlay = useCallback(() => {
		if (silencePlayingRef.current) {
			clearSilencePlaybackBound();
			stopSilenceSource();
			return;
		}
		setSilencePlaybackError(null);
		const activeSelection = selectionRef.current;
		if (loopSelectionRef.current && activeSelection[1] > activeSelection[0]) {
			silencePlaybackBoundRef.current = {
				stopMs: activeSelection[1],
				loopToMs: activeSelection[0],
			};
			setPreviewing(true);
			const currentPosition = silencePositionRef.current;
			const start =
				currentPosition >= activeSelection[0] &&
				currentPosition < activeSelection[1]
					? currentPosition
					: activeSelection[0];
			startSilenceAtRef.current(start, true);
			return;
		}
		clearSilencePlaybackBound();
		const start =
			silencePositionRef.current >= silenceDurationRef.current
				? 0
				: silencePositionRef.current;
		startSilenceAtRef.current(start, true);
	}, [clearSilencePlaybackBound, stopSilenceSource]);

	const activePosition = useCallback(
		() =>
			playbackTabRef.current === "silence"
				? silencePositionRef.current
				: positionRef.current,
		[],
	);
	const activeDuration = useCallback(
		() =>
			playbackTabRef.current === "silence"
				? silenceDurationRef.current
				: durationRef.current,
		[],
	);
	const storeActiveSelection = useCallback((next: SessionSelection) => {
		selectionRef.current = next;
		if (playbackTabRef.current === "silence") {
			silenceSelectionRef.current = next;
		} else {
			normalSelectionRef.current = next;
		}
		setSelection(next);
	}, []);

	const setSelectionEdgeFromPlayhead = useCallback(
		(edge: SelectionEdge) => {
			const current = selectionRef.current;
			const positionMs = activePosition();
			if (!canSetSelectionEdge(current, edge, positionMs)) {
				setSelectionHint(
					edge === "start"
						? "The playhead is at or beyond the right edge. Use O or E for that side."
						: "The playhead is at or before the left edge. Use I or E for that side.",
				);
				return;
			}
			if (playbackTabRef.current === "silence") {
				setSilenceSeekPreviewMs(null);
			} else {
				setSeekPreviewMs(null);
			}
			setSelectionHint(null);
			const next = applyEdge(current, edge, positionMs, activeDuration());
			storeActiveSelection(next);
		},
		[activeDuration, activePosition, storeActiveSelection],
	);

	const setNearestEdgeFromPlayhead = useCallback(() => {
		setSelectionEdgeFromPlayhead(
			nearestSelectionEdge(selectionRef.current, activePosition()),
		);
	}, [activePosition, setSelectionEdgeFromPlayhead]);

	const changeSelection = useCallback(
		(next: SessionSelection) => {
			setSelectionHint(null);
			storeActiveSelection(next);
		},
		[storeActiveSelection],
	);

	const resetSelection = useCallback(() => {
		if (playbackTabRef.current === "silence") {
			clearSilencePlaybackBound();
		} else {
			clearPlaybackBound();
		}
		setSelectionHint(null);
		const stampMs =
			playbackTabRef.current === "normal" &&
			stampClipRequested &&
			deepLinkedPositionMs !== null
				? deepLinkedPositionMs
				: undefined;
		const next = resetClipSelection(activeDuration(), stampMs);
		storeActiveSelection(next);
	}, [
		activeDuration,
		clearPlaybackBound,
		clearSilencePlaybackBound,
		deepLinkedPositionMs,
		stampClipRequested,
		storeActiveSelection,
	]);

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
			loopToMs: loopSelectionRef.current ? selectionRef.current[0] : null,
		};
		setPreviewing(true);
		setSeekPreviewMs(null);
		startAtRef.current(selectionRef.current[0], true);
	}, [clearPlaybackBound, stopSource]);

	const toggleSilencePreview = useCallback(() => {
		if (silencePlaybackBoundRef.current) {
			clearSilencePlaybackBound();
			stopSilenceSource();
			return;
		}
		if (selectionRef.current[1] <= selectionRef.current[0]) return;
		setSilencePlaybackError(null);
		silencePlaybackBoundRef.current = {
			stopMs: selectionRef.current[1],
			loopToMs: loopSelectionRef.current ? selectionRef.current[0] : null,
		};
		setPreviewing(true);
		setSilenceSeekPreviewMs(null);
		startSilenceAtRef.current(selectionRef.current[0], true);
	}, [clearSilencePlaybackBound, stopSilenceSource]);

	const toggleActivePlay = useCallback(() => {
		if (playbackTabRef.current === "silence") toggleSilencePlay();
		else togglePlay();
	}, [togglePlay, toggleSilencePlay]);

	const toggleActivePreview = useCallback(() => {
		if (playbackTabRef.current === "silence") toggleSilencePreview();
		else togglePreview();
	}, [togglePreview, toggleSilencePreview]);

	const seekActive = useCallback(
		(position: number) => {
			if (playbackTabRef.current === "silence") seekSilence(position);
			else seek(position);
		},
		[seek, seekSilence],
	);

	const changeLoopSelection = useCallback(
		(enabled: boolean) => {
			const activeSelection = selectionRef.current;
			if (enabled && activeSelection[1] <= activeSelection[0]) return;
			loopSelectionRef.current = enabled;
			setLoopSelection(enabled);
			if (playbackTabRef.current === "silence") {
				if (!enabled) {
					const bound = silencePlaybackBoundRef.current;
					if (bound) {
						silencePlaybackBoundRef.current = {
							...bound,
							loopToMs: null,
						};
					}
					return;
				}
				setSilenceSeekPreviewMs(null);
				if (silencePlaybackBoundRef.current) {
					silencePlaybackBoundRef.current = {
						stopMs: activeSelection[1],
						loopToMs: activeSelection[0],
					};
					setPreviewing(true);
					startSilenceAtRef.current(
						activeSelection[0],
						silencePlayingRef.current,
					);
					return;
				}
				clearSilencePlaybackBound();
				startSilenceAtRef.current(activeSelection[0], false);
				return;
			}
			if (!enabled) {
				const bound = playbackBoundRef.current;
				if (bound) playbackBoundRef.current = { ...bound, loopToMs: null };
				return;
			}

			setSeekPreviewMs(null);
			if (playbackBoundRef.current) {
				playbackBoundRef.current = {
					stopMs: activeSelection[1],
					loopToMs: activeSelection[0],
				};
				setPreviewing(true);
				startAtRef.current(activeSelection[0], playingRef.current);
				return;
			}

			clearPlaybackBound();
			startAtRef.current(activeSelection[0], false);
		},
		[clearPlaybackBound, clearSilencePlaybackBound],
	);

	// Keep a live preview bound aligned with selection edits. Moving the range
	// away from the playhead ends the preview and disables loop mode.
	useEffect(() => {
		if (playbackTab === "silence") {
			const playheadInsideSelection = selectionContainsPosition(
				selection,
				silencePositionRef.current,
			);
			if (loopSelection && !playheadInsideSelection) {
				clearSilencePlaybackBound();
				disableLoopSelection();
				return;
			}
			const bound = silencePlaybackBoundRef.current;
			if (!bound) return;
			if (!playheadInsideSelection) {
				clearSilencePlaybackBound();
				return;
			}
			silencePlaybackBoundRef.current = {
				stopMs: selection[1],
				loopToMs: loopSelection ? selectionRef.current[0] : null,
			};
			if (silencePositionRef.current >= selection[1]) {
				applySilencePlaybackBound(silencePositionRef.current);
			}
			return;
		}
		const playheadInsideSelection = selectionContainsPosition(
			selection,
			positionRef.current,
		);
		if (loopSelection && !playheadInsideSelection) {
			clearPlaybackBound();
			disableLoopSelection();
			return;
		}
		const bound = playbackBoundRef.current;
		if (!bound) return;
		if (!playheadInsideSelection) {
			clearPlaybackBound();
			return;
		}
		playbackBoundRef.current = {
			stopMs: selection[1],
			loopToMs: loopSelection ? selectionRef.current[0] : null,
		};
		if (positionRef.current >= selection[1]) {
			applyPlaybackBound(positionRef.current);
		}
	}, [
		applyPlaybackBound,
		applySilencePlaybackBound,
		clearPlaybackBound,
		clearSilencePlaybackBound,
		disableLoopSelection,
		loopSelection,
		playbackTab,
		selection,
	]);

	useEffect(() => {
		const handlePlaybackShortcut = (event: KeyboardEvent) => {
			if (
				isResetClipSelectionShortcut(event) &&
				!playbackShortcutTargetAcceptsText(event.target)
			) {
				event.preventDefault();
				resetSelection();
				return;
			}
			if (playbackShortcutTargetAcceptsText(event.target)) return;
			if (event.ctrlKey || event.metaKey || event.altKey) {
				if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
			}

			if (event.key === " " || event.code === "Space") {
				if (event.repeat) return;
				event.preventDefault();
				toggleActivePlay();
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

			if (event.key === "e" || event.key === "E") {
				if (event.repeat) return;
				event.preventDefault();
				setNearestEdgeFromPlayhead();
				return;
			}

			if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
			if (playbackShortcutTargetOwnsArrows(event.target)) return;
			event.preventDefault();
			const distance =
				playbackTab === "normal" && stampClipRequested
					? event.ctrlKey || event.metaKey
						? CLIP_CTRL_ARROW_SEEK_MS
						: event.shiftKey
							? CLIP_SHIFT_ARROW_SEEK_MS
							: CLIP_ARROW_SEEK_MS
					: event.ctrlKey || event.metaKey
						? CTRL_ARROW_SEEK_MS
						: ARROW_SEEK_MS;
			const direction = event.key === "ArrowRight" ? 1 : -1;
			if (playbackTab === "silence") setSilenceSeekPreviewMs(null);
			else setSeekPreviewMs(null);
			seekActive(activePosition() + direction * distance);
		};

		window.addEventListener("keydown", handlePlaybackShortcut);
		return () => window.removeEventListener("keydown", handlePlaybackShortcut);
	}, [
		resetSelection,
		playbackTab,
		activePosition,
		seekActive,
		setNearestEdgeFromPlayhead,
		setSelectionEdgeFromPlayhead,
		stampClipRequested,
		toggleActivePlay,
	]);

	const selectPlaybackTab = useCallback(
		(nextTab: "normal" | "silence") => {
			if (nextTab === playbackTabRef.current) return;
			setSelectionHint(null);
			disableLoopSelection();
			if (nextTab === "silence") {
				clearPlaybackBound();
				generationRef.current += 1;
				stopSource();
				playingRef.current = false;
				setPlaying(false);
			} else {
				clearSilencePlaybackBound();
				stopSilenceSource();
			}
			playbackTabRef.current = nextTab;
			const nextSelection =
				nextTab === "silence"
					? (silenceSelectionRef.current ??
						resetClipSelection(silenceDurationRef.current))
					: normalSelectionRef.current;
			if (nextTab === "silence") {
				silenceSelectionRef.current = nextSelection;
			}
			selectionRef.current = nextSelection;
			setSelection(nextSelection);
			setPlaybackTab(nextTab);
		},
		[
			clearPlaybackBound,
			clearSilencePlaybackBound,
			disableLoopSelection,
			stopSilenceSource,
			stopSource,
		],
	);

	const applySilenceRemovalStatus = useCallback(
		(result: SilenceRemovalStatus) => {
			setSilenceRemoval(result);
			if (result.status === "ready") {
				setSilenceFreeUrl(silenceFreeMediaUrl);
				if (
					silenceRemovalRequestedRef.current ||
					deepLink?.silenceFree === true
				) {
					silenceRemovalRequestedRef.current = false;
					if (!deepLink?.silenceFree) {
						setSessionMessage("Silence-free session ready.");
					}
					selectPlaybackTab("silence");
				}
				return;
			}
			if (playbackTabRef.current === "silence") {
				selectPlaybackTab("normal");
			}
			setSilenceFreeUrl(null);
			silenceDurationRef.current = 0;
			setSilenceDurationMs(0);
			setSilenceSeekPreviewMs(null);
			if (result.status === "failed") {
				silenceRemovalRequestedRef.current = false;
				setSessionError("Silence removal failed. You can try again.");
			}
		},
		[deepLink?.silenceFree, selectPlaybackTab, silenceFreeMediaUrl],
	);

	useEffect(() => {
		let cancelled = false;
		silenceRemovalRequestedRef.current = false;
		setSilenceFreeUrl(null);
		setSilenceRemoval({ status: "idle", progress: 0 });
		setSessionError(null);
		setSessionMessage(null);
		playbackTabRef.current = "normal";
		setPlaybackTab("normal");
		silenceSelectionRef.current = null;
		silenceDurationRef.current = 0;
		silencePositionRef.current = 0;
		setSilenceDurationMs(0);
		setSilencePositionMs(0);
		setSilenceSeekPreviewMs(null);
		setSilencePlaybackError(null);
		if (manifest?.state !== "finalized") return;

		void authedFetch(`audio/sessions/${props.sessionId}/remove-silence`)
			.then(async (response) => {
				if (!response.ok) return;
				const result = parseSilenceRemovalStatus(await response.json());
				if (!cancelled) applySilenceRemovalStatus(result);
			})
			.catch(() => {
				// A temporary status lookup failure leaves the action available.
			});
		return () => {
			cancelled = true;
		};
	}, [applySilenceRemovalStatus, manifest?.state, props.sessionId]);

	useEffect(() => {
		if (silenceRemoval.status !== "processing") return;
		let cancelled = false;
		let timeout: ReturnType<typeof globalThis.setTimeout> | undefined;

		const poll = async () => {
			try {
				const response = await authedFetch(
					`audio/sessions/${props.sessionId}/remove-silence`,
				);
				if (response.ok) {
					const result = parseSilenceRemovalStatus(await response.json());
					if (cancelled) return;
					applySilenceRemovalStatus(result);
					if (result.status !== "processing") return;
				}
			} catch {
				// Keep polling: the background job is independent of this request.
			}
			if (!cancelled) timeout = globalThis.setTimeout(poll, 1_000);
		};

		timeout = globalThis.setTimeout(poll, 750);
		return () => {
			cancelled = true;
			if (timeout !== undefined) globalThis.clearTimeout(timeout);
		};
	}, [applySilenceRemovalStatus, props.sessionId, silenceRemoval.status]);

	const downloadSession = async () => {
		setSessionAction("download");
		setActionError(null);
		setSessionError(null);
		setSessionMessage(null);
		try {
			const response = await authedFetch(
				`audio/sessions/${props.sessionId}/download`,
			);
			if (!response.ok) {
				setSessionError(`Session download failed (${response.status}).`);
				return;
			}
			saveBlob(await response.blob(), `session-${props.sessionId}.ogg`);
		} catch {
			setSessionError("Session download failed.");
		} finally {
			setSessionAction(null);
		}
	};

	const createSilenceFreeSession = async (force = false) => {
		setSessionAction("silence");
		silenceRemovalRequestedRef.current = true;
		setSilenceRemoval({ status: "processing", progress: 0 });
		setActionError(null);
		setSessionError(null);
		setSessionMessage(null);
		try {
			const response = await authedFetch(
				`audio/sessions/${props.sessionId}/remove-silence${force ? "?force=true" : ""}`,
				{
					method: "POST",
					headers: { "Content-Type": "application/json" },
					// Silence removal is a session action, never a Clip window range.
					body: JSON.stringify({}),
				},
			);
			if (!response.ok) {
				silenceRemovalRequestedRef.current = false;
				setSilenceRemoval({ status: "idle", progress: 0 });
				setSessionError(`Silence removal failed (${response.status}).`);
				return;
			}
			applySilenceRemovalStatus(
				parseSilenceRemovalStatus(await response.json()),
			);
		} catch {
			silenceRemovalRequestedRef.current = false;
			setSilenceRemoval({ status: "idle", progress: 0 });
			setSessionError("Silence removal failed.");
		} finally {
			setSessionAction(null);
		}
	};

	const downloadSilenceFreeSession = async () => {
		setSessionAction("silence-download");
		setSessionError(null);
		try {
			const response = await authedFetch(
				`audio/sessions/${props.sessionId}/silence-free?download=true`,
			);
			if (!response.ok) {
				setSessionError(`Silence-free download failed (${response.status}).`);
				return;
			}
			saveBlob(
				await response.blob(),
				`session-${props.sessionId}-silence-free.ogg`,
			);
		} catch {
			setSessionError("Silence-free download failed.");
		} finally {
			setSessionAction(null);
		}
	};

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

	return (
		<Box sx={{ pb: 4 }}>
			{silenceFreeUrl && (
				// biome-ignore lint/a11y/useMediaCaption: user voice recordings do not have a caption track
				<audio
					key={`${silenceFreeUrl}#${silenceRetryKey}`}
					ref={silenceAudioRef}
					crossOrigin="use-credentials"
					preload="metadata"
					src={silenceFreeUrl}
					onLoadedMetadata={(event) => {
						silenceMediaRetryRef.current = false;
						updateSilenceDuration(event.currentTarget);
					}}
					onDurationChange={(event) =>
						updateSilenceDuration(event.currentTarget)
					}
					onTimeUpdate={(event) => {
						const nextPosition = event.currentTarget.currentTime * 1_000;
						if (applySilencePlaybackBound(nextPosition)) return;
						silencePositionRef.current = nextPosition;
						setSilencePositionMs(nextPosition);
					}}
					onPlay={() => {
						silencePlayingRef.current = true;
						setSilencePlaying(true);
					}}
					onPause={() => {
						silencePlayingRef.current = false;
						setSilencePlaying(false);
					}}
					onEnded={() => {
						silencePlayingRef.current = false;
						setSilencePlaying(false);
						clearSilencePlaybackBound();
					}}
					onError={() => {
						silencePlayingRef.current = false;
						setSilencePlaying(false);
						clearSilencePlaybackBound();
						if (silenceMediaRetryRef.current) {
							setSilencePlaybackError(
								"Silence-free audio could not be loaded.",
							);
							return;
						}
						silenceMediaRetryRef.current = true;
						void refreshForMediaRetry().then((ok) => {
							if (ok) {
								setSilencePlaybackError(null);
								setSilenceRetryKey((key) => key + 1);
							} else {
								setSilencePlaybackError(SESSION_EXPIRED_MESSAGE);
							}
						});
					}}
					style={{ display: "none" }}
				/>
			)}
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
				<Tabs
					value={playbackTab}
					onChange={(_event, value: "normal" | "silence") =>
						selectPlaybackTab(value)
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
				</Tabs>

				<Box sx={{ display: playbackTab === "normal" ? "block" : "none" }}>
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

				{playbackTab === "silence" && silenceFreeUrl && (
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
			</Paper>

			<Paper ref={clipEditorRef} sx={{ p: 2, my: 2 }}>
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
					onPreview={toggleActivePreview}
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
					/>
					<Button
						variant="contained"
						onClick={() => void createSelectedClip()}
						disabled={clipState.isLoading}
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
