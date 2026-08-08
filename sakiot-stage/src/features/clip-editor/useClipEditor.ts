import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ClipEditorEngine } from "./engine";
import type { ClipEdit } from "./model";
import { emptyEdit, makeSegment, segmentDuration } from "./model";
import { loadClipBuffer } from "./useClipBuffer";
import { useEditHistory } from "./useEditHistory";

export type UseClipEditorReturn = ReturnType<typeof useClipEditor>;

export function useClipEditor() {
	const history = useEditHistory(emptyEdit());
	const { edit, preview, flush, apply, undo, redo, canUndo, canRedo } = history;
	const [positionSec, setPositionSec] = useState(0);
	const [playing, setPlaying] = useState(false);
	const [loop, setLoop] = useState(false);
	const [selectedSegmentId, setSelectedSegmentId] = useState<string | null>(
		null,
	);
	const [viewStartSec, setViewStartSec] = useState(0);
	const [viewWidthSec, setViewWidthSec] = useState(30);
	const [loadingClips, setLoadingClips] = useState<Map<string, boolean>>(
		new Map(),
	);

	const engineRef = useRef<ClipEditorEngine | null>(null);
	const buffersRef = useRef<Map<string, AudioBuffer>>(new Map());
	const positionRef = useRef(0);
	const playingRef = useRef(false);
	const loopRef = useRef(loop);
	const editRef = useRef(edit);
	const segmentsRef = useRef(edit.segments);
	const draggingRef = useRef(false);
	const rafRef = useRef<number | null>(null);

	useEffect(() => {
		editRef.current = edit;
	}, [edit]);
	useEffect(() => {
		loopRef.current = loop;
	}, [loop]);
	useEffect(() => {
		playingRef.current = playing;
	}, [playing]);

	const engine = useMemo(() => {
		if (!engineRef.current) engineRef.current = new ClipEditorEngine();
		return engineRef.current;
	}, []);

	useEffect(
		() => () => {
			if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
			engineRef.current?.dispose();
		},
		// Engine is created once per mount via the useMemo guard.
		// eslint-disable-next-line react-hooks/exhaustive-deps
		[],
	);

	const tick = useCallback(() => {
		if (!engine.isPlaying) {
			rafRef.current = null;
			setPlaying(false);
			positionRef.current = 0;
			setPositionSec(0);
			return;
		}
		const next = engine.positionSec;
		positionRef.current = next;
		setPositionSec(next);
		rafRef.current = requestAnimationFrame(tick);
	}, [engine]);

	const startPlayback = useCallback(
		(fromSec?: number) => {
			if (rafRef.current === null) rafRef.current = requestAnimationFrame(tick);
			setPlaying(true);
			engine.play(
				editRef.current,
				fromSec ?? positionRef.current,
				buffersRef.current,
				loopRef.current,
			);
		},
		[engine, tick],
	);

	const togglePlay = useCallback(() => {
		if (playingRef.current) {
			engine.stop();
			if (rafRef.current !== null) {
				cancelAnimationFrame(rafRef.current);
				rafRef.current = null;
			}
			setPlaying(false);
			positionRef.current = 0;
			setPositionSec(0);
			return;
		}
		startPlayback();
	}, [engine, startPlayback]);

	const seek = useCallback(
		(sec: number) => {
			const clamped = Math.max(0, sec);
			if (playingRef.current) {
				engine.seekTo(
					editRef.current,
					clamped,
					buffersRef.current,
					loopRef.current,
				);
			}
			positionRef.current = clamped;
			setPositionSec(clamped);
		},
		[engine],
	);

	const setLooping = useCallback(
		(next: boolean) => {
			setLoop(next);
			loopRef.current = next;
			if (playingRef.current) {
				engine.seekTo(
					editRef.current,
					positionRef.current,
					buffersRef.current,
					next,
				);
			}
		},
		[engine],
	);

	const setMasterVolume = useCallback(
		(db: number) => {
			apply((current) => ({ ...current, masterVolumeDb: db }));
			engine.setMasterVolume(db);
		},
		[apply, engine],
	);

	const registerBuffer = useCallback(
		(sourceId: string, buffer: AudioBuffer) => {
			buffersRef.current.set(sourceId, buffer);
		},
		[],
	);

	const loadClip = useCallback(
		(
			guildId: string,
			clipId: string,
			lengthSec: number,
			track: number,
			timelineStart?: number,
		) => {
			if (buffersRef.current.has(clipId)) {
				apply((current) =>
					addSegmentAt(
						current,
						clipId,
						lengthSec,
						track,
						timelineStart ?? endOfTrack(current, track),
					),
				);
				return;
			}
			setLoadingClips((previous) => new Map(previous).set(clipId, true));
			loadClipBuffer(guildId, clipId)
				.then((buffer) => {
					registerBuffer(clipId, buffer);
					apply((current) =>
						addSegmentAt(
							current,
							clipId,
							lengthSec,
							track,
							timelineStart ?? endOfTrack(current, track),
						),
					);
				})
				.catch(() => {
					// The bin stays clickable; a failed decode just adds nothing.
				})
				.finally(() => {
					setLoadingClips((previous) => {
						const next = new Map(previous);
						next.delete(clipId);
						return next;
					});
				});
		},
		[apply, registerBuffer],
	);

	useEffect(() => {
		const selected = edit.segments.find((s) => s.id === selectedSegmentId);
		if (selected) {
			engine.applySegmentEffects(selected.id, selected.effects);
		}
	}, [edit.segments, engine, selectedSegmentId]);

	useEffect(() => {
		const segmentsChanged = segmentsRef.current !== edit.segments;
		segmentsRef.current = edit.segments;
		if (segmentsChanged && !draggingRef.current && playingRef.current) {
			engine.play(
				editRef.current,
				positionRef.current,
				buffersRef.current,
				loopRef.current,
			);
		}
	}, [edit, engine]);

	const select = useCallback(
		(id: string | null) => setSelectedSegmentId(id),
		[],
	);

	const beginGesture = useCallback(() => {
		draggingRef.current = true;
	}, []);

	const endGesture = useCallback(() => {
		draggingRef.current = false;
		flush();
	}, [flush]);

	const zoom = useCallback(
		(factor: number) => {
			const center = positionRef.current;
			const nextWidth = Math.max(1, Math.min(120, viewWidthSec * factor));
			const ratio = nextWidth / viewWidthSec;
			setViewStartSec(Math.max(0, center - (center - viewStartSec) * ratio));
			setViewWidthSec(nextWidth);
		},
		[viewStartSec, viewWidthSec],
	);

	const fitView = useCallback(() => {
		const duration = editRef.current.segments.reduce(
			(max, segment) =>
				Math.max(max, segment.timelineStart + segmentDuration(segment)),
			0,
		);
		setViewStartSec(0);
		setViewWidthSec(Math.max(5, duration + 2));
	}, []);

	const removeSelected = useCallback(() => {
		if (!selectedSegmentId) return;
		const id = selectedSegmentId;
		apply((current) => ({
			...current,
			segments: current.segments.filter((s) => s.id !== id),
		}));
		setSelectedSegmentId(null);
	}, [apply, selectedSegmentId]);

	const splitSelectedAtPlayhead = useCallback(() => {
		if (!selectedSegmentId) return;
		const id = selectedSegmentId;
		apply((current) => {
			const segment = current.segments.find((s) => s.id === id);
			if (!segment) return current;
			const at = positionRef.current;
			const start = segment.timelineStart;
			const end = start + segmentDuration(segment);
			if (at - start < 0.05 || end - at < 0.05) return current;
			return {
				...current,
				segments: [
					...current.segments,
					{
						...segment,
						id: `seg-split-${Date.now()}`,
						sourceIn: segment.sourceIn + (at - start) * segment.effects.rate,
						timelineStart: at,
					},
				],
			};
		});
	}, [apply, selectedSegmentId]);

	const selectedSegment =
		edit.segments.find((s) => s.id === selectedSegmentId) ?? null;

	const sourceDuration = useCallback((sourceId: string): number | null => {
		return buffersRef.current.get(sourceId)?.duration ?? null;
	}, []);

	return {
		edit,
		preview,
		flush,
		apply,
		undo,
		redo,
		canUndo,
		canRedo,
		positionSec,
		setPosition: seek,
		playing,
		togglePlay,
		loop,
		setLooping,
		masterVolumeDb: edit.masterVolumeDb,
		setMasterVolume,
		selectedSegmentId,
		selectedSegment,
		select,
		loadClip,
		registerBuffer,
		sourceDuration,
		loadingClips,
		viewStartSec,
		viewWidthSec,
		zoom,
		fitView,
		beginGesture,
		endGesture,
		removeSelected,
		splitSelectedAtPlayhead,
	};
}

function endOfTrack(edit: ClipEdit, track: number): number {
	return edit.segments.reduce(
		(max, segment) =>
			segment.track === track
				? Math.max(max, segment.timelineStart + segmentDuration(segment))
				: max,
		0,
	);
}

function addSegmentAt(
	edit: ClipEdit,
	clipId: string,
	lengthSec: number,
	track: number,
	timelineStart: number,
): ClipEdit {
	const segment = makeSegment(
		"clip",
		clipId,
		0,
		lengthSec,
		timelineStart,
		track,
	);
	return {
		...edit,
		segments: [...edit.segments, segment],
		tracks: Math.max(edit.tracks, track + 1),
	};
}
