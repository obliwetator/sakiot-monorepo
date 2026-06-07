import type Hls from "hls.js";
import { useEffect, useRef, useState } from "react";
import { useDispatch } from "react-redux";
import { useLocation, useParams } from "react-router-dom";
import {
	BASE_API_URL,
	useCheckSilenceFileQuery,
	useGetLiveStateQuery,
	useGetRecordingEventsQuery,
} from "../../app/apiSlice";
import { useAppSelector } from "../../app/hooks";
import type { AudioParams, UserGuilds } from "../../Constants";
import { setHasSilence } from "../../reducers/silence";
import { RangeSlider } from "./RangeSlider";

export function AudioInterface(props: {
	isClip: boolean;
	userGuilds: UserGuilds[] | null;
	isSilence: boolean;
	absoluteStartMs?: number | null;
}) {
	const intervalRef = useRef<number | undefined>(undefined);
	const params = useParams<AudioParams>();
	const location = useLocation();
	const [audioRef, setAudioRef] = useState<HTMLAudioElement | null>(null);
	const [readyToPlay, setReadyToPlay] = useState(false);
	const [error, setError] = useState(false);
	// `hls` path = streaming via HLS, `blob` path = full-file .ogg.
	// HLS only earns its keep while a recording is still growing — for
	// finished recordings the .ogg is complete and the blob path works
	// directly. Clips/silence always blob. `hlsFailed` sticks the choice
	// at blob if HLS errors out for a live recording.
	const [hlsFailed, setHlsFailed] = useState(false);
	const [trueDuration, setTrueDuration] = useState<number | null>(null);
	const dispatch = useDispatch();
	const value = useAppSelector((state) => state.hasSilence.value);
	const silenceVersion = useAppSelector((state) => state.hasSilence.version);

	const shouldCheckSilence =
		!props.isClip && !props.isSilence && !!params.file_name;
	const { isSuccess, isError } = useCheckSilenceFileQuery(
		{
			guild_id: params.guild_id ?? "",
			channel_id: params.channel_id ?? "",
			year: params.year ?? "",
			month: Number(params.month ?? ""),
			file_name: params.file_name ?? "",
		},
		{ skip: !shouldCheckSilence },
	);

	useEffect(() => {
		if (shouldCheckSilence) {
			if (isSuccess) {
				dispatch(setHasSilence(true));
			} else if (isError) {
				dispatch(setHasSilence(false));
			}
		}

		return () => {
			if (!props.isSilence) {
				dispatch(setHasSilence(false));
			}
		};
	}, [shouldCheckSilence, isSuccess, isError, dispatch, props.isSilence]);

	const recordingStateArgs =
		!props.isClip && !!params.file_name
			? {
					guild_id: params.guild_id ?? "",
					channel_id: params.channel_id ?? "",
					year: params.year ?? "",
					month: Number(params.month ?? ""),
					file_name: params.file_name ?? "",
				}
			: undefined;
	const { data: recordingState } = useGetLiveStateQuery(
		recordingStateArgs ?? ({} as never),
		{
			skip: !recordingStateArgs,
			pollingInterval: recordingStateArgs ? 10_000 : 0,
		},
	);
	const isLive = !!recordingState?.live;

	const eventsArgs =
		!props.isClip && !props.isSilence && !!params.file_name
			? {
					guild_id: params.guild_id ?? "",
					channel_id: params.channel_id ?? "",
					year: params.year ?? "",
					month: Number(params.month ?? ""),
					file_name: params.file_name ?? "",
					user_id: params.file_name?.split("-")[1],
				}
			: undefined;
	const { data: voiceEvents } = useGetRecordingEventsQuery(
		eventsArgs ?? ({} as never),
		{ skip: !eventsArgs },
	);
	const mode: "hls" | "blob" =
		props.isClip || props.isSilence || hlsFailed
			? "blob"
			: isLive
				? "hls"
				: "blob";

	// Direct streaming URL — browser issues HTTP Range requests against
	// the audio element so playback can start before the full file lands.
	const streamUrl = props.isClip
		? `${BASE_API_URL}audio/clips/${params.guild_id}/${encodeURIComponent(params.file_name ?? "")}`
		: `${BASE_API_URL}audio/${params.guild_id}/${params.channel_id}/${params.year}/${params.month}/${encodeURIComponent(params.file_name ?? "")}.ogg${props.isSilence ? `?silence=true&v=${silenceVersion}` : ""}`;

	// For non-clip/silence files, wait for liveState before opening the
	// stream — avoids racing a blob-style download against a live
	// recording that should have used HLS instead.
	const recordingStateResolved =
		props.isClip ||
		props.isSilence ||
		recordingState !== undefined ||
		hlsFailed;
	const shouldStream =
		mode === "blob" &&
		recordingStateResolved &&
		!(props.isSilence && !value) &&
		!!params.file_name;

	// HLS playback
	useEffect(() => {
		if (mode !== "hls" || !params.file_name || props.isClip || props.isSilence)
			return;

		setReadyToPlay(false);
		setError(false);
		setTrueDuration(null);

		const hlsUrl = `${BASE_API_URL}audio/live/${params.guild_id}/${params.channel_id}/${params.year}/${params.month}/${encodeURIComponent(params.file_name)}/playlist.m3u8`;

		const audio = new Audio();
		audio.preload = "auto";
		let hls: Hls | null = null;
		let isActive = true;
		let didFallback = false;

		const fallback = (reason: string) => {
			if (!isActive || didFallback) return;
			didFallback = true;
			console.warn("HLS failed, falling back to blob:", reason);
			hls?.destroy();
			hls = null;
			setHlsFailed(true);
		};

		const onDurationChange = () => {
			if (!isActive) return;
			if (Number.isFinite(audio.duration)) setTrueDuration(audio.duration);
		};
		const onCanPlay = () => {
			if (!isActive) return;
			const t = new URLSearchParams(location.search).get("t");
			if (t && audio.currentTime === 0) audio.currentTime = parseFloat(t);
			setReadyToPlay(true);
			setAudioRef(audio);
			if (Number.isFinite(audio.duration)) setTrueDuration(audio.duration);
		};
		audio.addEventListener("durationchange", onDurationChange);
		audio.addEventListener("canplay", onCanPlay);
		audio.addEventListener("error", () => fallback("audio element error"));

		// Safari natively decodes HLS and returns "probably" — skip the
		// hls.js download. Some Chromium builds return "maybe" without
		// actually being able to decode HLS, which lands in the native
		// path and immediately fails; require "probably" so those
		// browsers fall through to hls.js.
		if (audio.canPlayType("application/vnd.apple.mpegurl") === "probably") {
			audio.src = hlsUrl;
		} else {
			// Code-split hls.js so it doesn't bloat the initial bundle. Loaded
			// only when an HLS-eligible recording is opened.
			import("hls.js")
				.then(({ default: Hls }) => {
					if (!isActive) return;
					if (!Hls.isSupported()) {
						fallback("hls not supported in this browser");
						return;
					}
					hls = new Hls({
						xhrSetup: (xhr) => {
							xhr.withCredentials = true;
						},
						liveSyncDuration: 2,
						liveMaxLatencyDuration: Number.MAX_SAFE_INTEGER,
					});
					hls.on(Hls.Events.ERROR, (_e, data) => {
						if (data.fatal) fallback(`hls fatal: ${data.type}/${data.details}`);
					});
					hls.loadSource(hlsUrl);
					hls.attachMedia(audio);
				})
				.catch((e) => fallback(`hls.js import failed: ${e}`));
		}

		return () => {
			isActive = false;
			hls?.destroy();
			audio.pause();
			audio.removeAttribute("src");
			audio.load();
		};
	}, [
		mode,
		params.file_name,
		params.guild_id,
		params.channel_id,
		params.year,
		params.month,
		props.isClip,
		props.isSilence,
		location.search,
	]);

	// Streaming playback. Audio element pulls bytes via HTTP Range so
	// playback starts as soon as enough is buffered — no full blob download.
	// `crossOrigin = use-credentials` lets cookies ride along for auth.
	useEffect(() => {
		if (mode !== "blob") return;
		if (!shouldStream) {
			setReadyToPlay(false);
			setAudioRef(null);
			return;
		}

		setReadyToPlay(false);
		setError(false);
		setTrueDuration(null);

		const localAudioRef: HTMLAudioElement = new Audio();
		localAudioRef.crossOrigin = "use-credentials";
		localAudioRef.preload = "auto";
		localAudioRef.src = streamUrl;
		let isActive = true;

		localAudioRef.addEventListener("durationchange", () => {
			if (!isActive) return;
			if (
				localAudioRef.duration !== Infinity &&
				Number.isFinite(localAudioRef.duration)
			) {
				setTrueDuration(localAudioRef.duration);
			}
		});

		localAudioRef.addEventListener("canplay", () => {
			if (!isActive) return;
			const t = new URLSearchParams(location.search).get("t");
			if (t && localAudioRef.currentTime === 0)
				localAudioRef.currentTime = parseFloat(t);
			setReadyToPlay(true);
			setAudioRef(localAudioRef);
			if (
				localAudioRef.duration !== Infinity &&
				Number.isFinite(localAudioRef.duration)
			) {
				setTrueDuration(localAudioRef.duration);
			}
		});

		localAudioRef.onerror = () => {
			if (!isActive) return;
			setError(true);
		};

		return () => {
			isActive = false;
			localAudioRef.pause();
			localAudioRef.removeAttribute("src");
			localAudioRef.load();
		};
	}, [mode, shouldStream, streamUrl, location.search]);

	if (props.isSilence && !value) {
		return (
			<div className="w-full mt-4 text-gray-400">
				Audio file doesn't have a silence free version. You can generate one
				using the 'Remove Silence' button above.
			</div>
		);
	}

	if (error) {
		return (
			<div className="w-full">
				{props.isSilence
					? "Audio file doesn't have silence free vesion"
					: "An error occured fetching the audio. Try refreshing"}
			</div>
		);
	}

	return (
		<div className="w-full">
			{readyToPlay && audioRef ? (
				<>
					{mode === "hls" && isLive && (
						<div className="mb-2 inline-block px-2 py-1 text-xs font-bold text-white bg-red-600 rounded">
							● LIVE
						</div>
					)}
					<RangeSlider
						audioRef={audioRef}
						intervalRef={intervalRef}
						isClip={props.isClip}
						userGuilds={props.userGuilds}
						isSilence={props.isSilence}
						isLive={isLive}
						trueDuration={trueDuration}
						liveStartedAt={
							mode === "hls" && isLive
								? (recordingState?.started_at ?? null)
								: null
						}
						recordingStartedAtMs={
							props.absoluteStartMs ?? recordingState?.started_at ?? null
						}
						voiceEvents={voiceEvents}
					/>
				</>
			) : (
				"Loading Audio"
			)}
		</div>
	);
}
