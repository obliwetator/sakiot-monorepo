import {
	type MutableRefObject,
	type RefObject,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import {
	applyEdge,
	canSetSelectionEdge,
	nearestSelectionEdge,
	type SelectionEdge,
} from "./clipSelection";
import { isResetClipSelectionShortcut } from "./clipSelectionShortcuts";
import type { SessionDeepLink } from "./logicalSessionPlaybackState";
import {
	reconcileSessionSelection,
	resetClipSelection,
	type SelectionManifest,
	type SessionSelection,
	selectionAroundStamp,
} from "./logicalSessionSelection";
import {
	playbackShortcutTargetAcceptsText,
	playbackShortcutTargetOwnsArrows,
} from "./playbackShortcuts";
import type { SegmentedSessionPlayback } from "./useSegmentedSessionPlayback";
import type { SilenceFreePlayback } from "./useSilenceFreePlayback";

const ARROW_SEEK_MS = 5_000;
const CTRL_ARROW_SEEK_MS = 30_000;
const CLIP_ARROW_SEEK_MS = 100;
const CLIP_SHIFT_ARROW_SEEK_MS = 1_000;
const CLIP_CTRL_ARROW_SEEK_MS = 5_000;

function selectionsMatch(
	left: SessionSelection,
	right: SessionSelection,
): boolean {
	return left[0] === right[0] && left[1] === right[1];
}

interface SelectionControllerOptions {
	sessionId: string;
	manifest: SelectionManifest | null;
	deepLink: SessionDeepLink | null;
	normal: SegmentedSessionPlayback;
	silence: SilenceFreePlayback;
	clipEditorRef: RefObject<HTMLDivElement | null>;
	loopDisableRef: MutableRefObject<() => void>;
}

export function useSessionSelectionController(
	options: SelectionControllerOptions,
) {
	const [selection, setSelection] = useState<SessionSelection>([0, 0]);
	const [playbackTab, setPlaybackTab] = useState<"normal" | "silence">(
		"normal",
	);
	const [loopSelection, setLoopSelection] = useState(false);
	const [selectionHint, setSelectionHint] = useState<string | null>(null);
	const selectionRef = useRef<SessionSelection>([0, 0]);
	const normalSelectionRef = useRef<SessionSelection>([0, 0]);
	const silenceSelectionRef = useRef<SessionSelection | null>(null);
	const playbackTabRef = useRef<"normal" | "silence">("normal");
	const loopSelectionRef = useRef(false);
	const selectionManifestRef = useRef<SelectionManifest | null>(null);
	const previousSilenceDurationRef = useRef(0);
	const appliedDeepLinkRef = useRef<string | null>(null);

	useEffect(() => {
		selectionRef.current = selection;
	}, [selection]);
	useEffect(() => {
		loopSelectionRef.current = loopSelection;
	}, [loopSelection]);
	useEffect(() => {
		if (!selectionHint) return;
		const timeout = globalThis.setTimeout(() => setSelectionHint(null), 4_000);
		return () => globalThis.clearTimeout(timeout);
	}, [selectionHint]);

	const disableLoop = useCallback(() => {
		loopSelectionRef.current = false;
		setLoopSelection(false);
	}, []);
	options.loopDisableRef.current = disableLoop;

	const storeActiveSelection = useCallback((next: SessionSelection) => {
		selectionRef.current = next;
		if (playbackTabRef.current === "silence") {
			silenceSelectionRef.current = next;
		} else {
			normalSelectionRef.current = next;
		}
		setSelection(next);
	}, []);

	const manifestRecordingSessionId = options.manifest?.recordingSessionId;
	const manifestDurationMs = options.manifest?.durationMs;
	useEffect(() => {
		if (
			manifestRecordingSessionId === undefined ||
			manifestDurationMs === undefined
		) {
			return;
		}
		const manifest: SelectionManifest = {
			recordingSessionId: manifestRecordingSessionId,
			durationMs: manifestDurationMs,
		};
		const previous = selectionManifestRef.current;
		selectionManifestRef.current = manifest;
		const next = reconcileSessionSelection(
			normalSelectionRef.current,
			previous,
			manifest,
		);
		normalSelectionRef.current = next;
		if (playbackTabRef.current === "normal") {
			selectionRef.current = next;
			setSelection((current) =>
				selectionsMatch(current, next) ? current : next,
			);
		}
	}, [manifestDurationMs, manifestRecordingSessionId]);

	useEffect(() => {
		const duration = options.silence.durationMs;
		if (duration <= 0) return;
		const previousDuration = previousSilenceDurationRef.current;
		const current = silenceSelectionRef.current;
		const next = reconcileSessionSelection(
			current ?? [0, 0],
			current && previousDuration > 0
				? { recordingSessionId: "silence-free", durationMs: previousDuration }
				: null,
			{ recordingSessionId: "silence-free", durationMs: duration },
		);
		previousSilenceDurationRef.current = duration;
		silenceSelectionRef.current = next;
		if (playbackTabRef.current === "silence") {
			selectionRef.current = next;
			setSelection(next);
		}
		if (options.deepLink?.silenceFree) {
			const key = `${options.sessionId}:${options.deepLink.positionMs}:silence-free`;
			if (appliedDeepLinkRef.current !== key) {
				appliedDeepLinkRef.current = key;
				options.silence.startAt(
					Math.min(options.deepLink.positionMs, duration),
					false,
				);
			}
		}
	}, [
		options.deepLink,
		options.sessionId,
		options.silence.durationMs,
		options.silence.startAt,
	]);

	useEffect(() => {
		const manifest = options.manifest;
		const deepLink = options.deepLink;
		if (!manifest || !deepLink || deepLink.silenceFree) return;
		const key = `${options.sessionId}:${deepLink.positionMs}:${deepLink.fromStamp}`;
		if (appliedDeepLinkRef.current === key) return;
		appliedDeepLinkRef.current = key;
		const position = Math.min(deepLink.positionMs, manifest.durationMs);
		options.normal.clearBound();
		disableLoop();
		options.normal.startAt(position, false);
		if (!deepLink.fromStamp) return;
		const next = selectionAroundStamp(position, manifest.durationMs);
		normalSelectionRef.current = next;
		selectionRef.current = next;
		setSelection(next);
		requestAnimationFrame(() => {
			options.clipEditorRef.current?.scrollIntoView({
				behavior: "smooth",
				block: "center",
			});
		});
	}, [
		disableLoop,
		options.clipEditorRef,
		options.deepLink,
		options.manifest,
		options.normal.clearBound,
		options.normal.startAt,
		options.sessionId,
	]);

	const activePosition = useCallback(
		() =>
			playbackTabRef.current === "silence"
				? options.silence.positionMs
				: options.normal.positionMs,
		[options.normal.positionMs, options.silence.positionMs],
	);
	const activeDuration = useCallback(
		() =>
			playbackTabRef.current === "silence"
				? options.silence.durationMs
				: (options.manifest?.durationMs ?? 0),
		[options.manifest?.durationMs, options.silence.durationMs],
	);

	const setSelectionEdgeFromPlayhead = useCallback(
		(edge: SelectionEdge) => {
			const current = selectionRef.current;
			const position = activePosition();
			if (!canSetSelectionEdge(current, edge, position)) {
				setSelectionHint(
					edge === "start"
						? "The playhead is at or beyond the right edge. Use O or E for that side."
						: "The playhead is at or before the left edge. Use I or E for that side.",
				);
				return;
			}
			if (playbackTabRef.current === "silence") {
				options.silence.setSeekPreviewMs(null);
			} else {
				options.normal.setSeekPreviewMs(null);
			}
			setSelectionHint(null);
			storeActiveSelection(
				applyEdge(current, edge, position, activeDuration()),
			);
		},
		[
			activeDuration,
			activePosition,
			options.normal,
			options.silence,
			storeActiveSelection,
		],
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
		const active =
			playbackTabRef.current === "silence" ? options.silence : options.normal;
		active.clearBound();
		setSelectionHint(null);
		const stampMs =
			playbackTabRef.current === "normal" && options.deepLink?.fromStamp
				? options.deepLink.positionMs
				: undefined;
		storeActiveSelection(resetClipSelection(activeDuration(), stampMs));
	}, [
		activeDuration,
		options.deepLink,
		options.normal,
		options.silence,
		storeActiveSelection,
	]);

	const seekActive = useCallback(
		(position: number) => {
			setSelectionHint(null);
			if (playbackTabRef.current === "silence") {
				options.silence.seek(
					position,
					selectionRef.current,
					loopSelectionRef.current,
				);
			} else {
				options.normal.seek(
					position,
					selectionRef.current,
					loopSelectionRef.current,
				);
			}
		},
		[options.normal, options.silence],
	);

	const toggleActivePlay = useCallback(() => {
		const active =
			playbackTabRef.current === "silence" ? options.silence : options.normal;
		active.togglePlay(selectionRef.current, loopSelectionRef.current);
	}, [options.normal, options.silence]);

	const toggleActivePreview = useCallback(() => {
		const active =
			playbackTabRef.current === "silence" ? options.silence : options.normal;
		active.togglePreview(selectionRef.current, loopSelectionRef.current);
	}, [options.normal, options.silence]);

	const changeLoopSelection = useCallback(
		(enabled: boolean) => {
			if (enabled && selectionRef.current[1] <= selectionRef.current[0]) return;
			loopSelectionRef.current = enabled;
			setLoopSelection(enabled);
			const active =
				playbackTabRef.current === "silence" ? options.silence : options.normal;
			active.updateLoop(enabled, selectionRef.current);
		},
		[options.normal, options.silence],
	);

	useEffect(() => {
		const active = playbackTab === "silence" ? options.silence : options.normal;
		active.syncBound(selection, loopSelection);
	}, [loopSelection, options.normal, options.silence, playbackTab, selection]);

	const selectPlaybackTab = useCallback(
		(nextTab: "normal" | "silence") => {
			if (nextTab === playbackTabRef.current) return;
			setSelectionHint(null);
			disableLoop();
			if (nextTab === "silence") options.normal.stop();
			else options.silence.stop();
			playbackTabRef.current = nextTab;
			const next =
				nextTab === "silence"
					? (silenceSelectionRef.current ??
						resetClipSelection(options.silence.durationMs))
					: normalSelectionRef.current;
			if (nextTab === "silence") silenceSelectionRef.current = next;
			selectionRef.current = next;
			setSelection(next);
			setPlaybackTab(nextTab);
		},
		[disableLoop, options.normal, options.silence],
	);

	useEffect(() => {
		const handleShortcut = (event: KeyboardEvent) => {
			if (
				isResetClipSelectionShortcut(event) &&
				!playbackShortcutTargetAcceptsText(event.target)
			) {
				event.preventDefault();
				resetSelection();
				return;
			}
			if (playbackShortcutTargetAcceptsText(event.target)) return;
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
			if (event.altKey) return;
			event.preventDefault();
			const stampMode = playbackTab === "normal" && options.deepLink?.fromStamp;
			const distance = stampMode
				? event.ctrlKey || event.metaKey
					? CLIP_CTRL_ARROW_SEEK_MS
					: event.shiftKey
						? CLIP_SHIFT_ARROW_SEEK_MS
						: CLIP_ARROW_SEEK_MS
				: event.ctrlKey || event.metaKey
					? CTRL_ARROW_SEEK_MS
					: ARROW_SEEK_MS;
			seekActive(
				activePosition() + (event.key === "ArrowRight" ? distance : -distance),
			);
		};
		window.addEventListener("keydown", handleShortcut);
		return () => window.removeEventListener("keydown", handleShortcut);
	}, [
		activePosition,
		options.deepLink?.fromStamp,
		playbackTab,
		resetSelection,
		seekActive,
		setNearestEdgeFromPlayhead,
		setSelectionEdgeFromPlayhead,
		toggleActivePlay,
	]);

	return {
		selection,
		playbackTab,
		loopSelection,
		selectionHint,
		previewing:
			playbackTab === "silence"
				? options.silence.boundActive
				: options.normal.boundActive,
		changeSelection,
		resetSelection,
		seekActive,
		toggleActivePlay,
		toggleActivePreview,
		changeLoopSelection,
		setSelectionEdgeFromPlayhead,
		setNearestEdgeFromPlayhead,
		selectPlaybackTab,
		disableLoop,
	};
}

export type SessionSelectionController = ReturnType<
	typeof useSessionSelectionController
>;
