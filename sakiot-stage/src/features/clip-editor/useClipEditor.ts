import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ClipEditorEngine } from "./engine";
import { isInspectorFeatureDisabled } from "./inspectorFeaturePolicy";
import {
	type ClipEdit,
	cloneTimelineSegments,
	duplicateTimelineSegments,
	editDuration,
	emptyEdit,
	expandMergeGroups,
	isPastePlacementLegal,
	MERGE_BLOCK_MESSAGES,
	makeSegment,
	mergeBlockReason,
	mergeSegments,
	removeTrack as removeTrackFromEdit,
	segmentDuration,
	segmentsForCopy,
	snapToPastedLayout,
	splitSegment,
	type TimelineSegment,
	toggleTrackMute as toggleTrackMuteInEdit,
	unmergeSegments,
} from "./model";
import { sharedDspPreprocessKey } from "./sharedDsp";
import { loadClipBuffer } from "./useClipBuffer";
import { useEditHistory } from "./useEditHistory";

export type UseClipEditorReturn = ReturnType<typeof useClipEditor>;

export interface PasteTarget {
	startSec: number;
	track: number;
}

export function useClipEditor(options: { copyAllSelected?: boolean } = {}) {
	const copyAllSelected = options.copyAllSelected ?? true;
	const history = useEditHistory(emptyEdit());
	const { edit, preview, flush, apply, undo, redo, canUndo, canRedo, reset } =
		history;
	const [positionSec, setPositionSec] = useState(0);
	const [playing, setPlaying] = useState(false);
	const [loop, setLoop] = useState(false);
	const [selectedSegmentIds, setSelectedSegmentIds] = useState<string[]>([]);
	/** Track that clicks on the timeline activate; paste and bin adds land here. */
	const [activeTrack, setActiveTrack] = useState(0);
	const [clipboard, setClipboard] = useState<TimelineSegment[] | null>(null);
	/** Segments whose data is in the clipboard; they show the copied ring. */
	const [copySourceIds, setCopySourceIds] = useState<string[]>([]);
	/** Warning describing why the last merge attempt was refused. */
	const [mergeWarning, setMergeWarning] = useState<string | null>(null);
	const [viewStartSec, setViewStartSec] = useState(0);
	const [viewWidthSec, setViewWidthSec] = useState(30);
	const contentDurationSec = editDuration(edit) + 2;
	const timelineDurationSec = Math.max(viewWidthSec, contentDurationSec);
	const viewMaxStartSec = Math.max(0, timelineDurationSec - viewWidthSec);
	const [loadingClips, setLoadingClips] = useState<Map<string, boolean>>(
		new Map(),
	);

	const engineRef = useRef<ClipEditorEngine | null>(null);
	const buffersRef = useRef<Map<string, AudioBuffer>>(new Map());
	const positionRef = useRef(0);
	const playingRef = useRef(false);
	const loopRef = useRef(loop);
	const editRef = useRef(edit);
	const playbackStructureRef = useRef(playbackStructureKey(edit));
	const draggingRef = useRef(false);
	const pasteTargetRef = useRef<PasteTarget | null>(null);
	const rafRef = useRef<number | null>(null);
	const playbackRefreshRef = useRef<number | null>(null);

	useEffect(() => {
		editRef.current = edit;
	}, [edit]);
	useEffect(() => {
		loopRef.current = loop;
	}, [loop]);
	useEffect(() => {
		playingRef.current = playing;
	}, [playing]);

	useEffect(() => {
		setViewStartSec((current) =>
			Math.min(viewMaxStartSec, Math.max(0, current)),
		);
	}, [viewMaxStartSec]);

	const engine = useMemo(() => {
		if (!engineRef.current) engineRef.current = new ClipEditorEngine();
		return engineRef.current;
	}, []);

	useEffect(
		() => () => {
			if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
			if (playbackRefreshRef.current !== null) {
				clearTimeout(playbackRefreshRef.current);
			}
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
		if (engine.isPlaying) {
			engine.pause();
			if (rafRef.current !== null) {
				cancelAnimationFrame(rafRef.current);
				rafRef.current = null;
			}
			setPlaying(false);
			positionRef.current = engine.positionSec;
			setPositionSec(engine.positionSec);
			return;
		}
		startPlayback();
	}, [engine, startPlayback]);

	const seek = useCallback(
		(sec: number) => {
			const clamped = Math.max(0, sec);
			if (engine.isPlaying) {
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
			if (engine.isPlaying) {
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

	const toggleTrackMute = useCallback(
		(track: number) => {
			apply((current) => toggleTrackMuteInEdit(current, track));
		},
		[apply],
	);

	const removeTrack = useCallback(
		(track: number) => {
			const current = editRef.current;
			if (current.tracks <= 1 || track < 0 || track >= current.tracks) return;
			const removedIds = new Set(
				current.segments
					.filter((segment) => segment.track === track)
					.map((segment) => segment.id),
			);
			apply((edit) => removeTrackFromEdit(edit, track));
			setSelectedSegmentIds((ids) => ids.filter((id) => !removedIds.has(id)));
			setActiveTrack((active) =>
				active > track ? active - 1 : Math.min(active, current.tracks - 2),
			);
		},
		[apply],
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
				return Promise.resolve(true);
			}
			setLoadingClips((previous) => new Map(previous).set(clipId, true));
			return loadClipBuffer(guildId, clipId)
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
					return true;
				})
				.catch(() => {
					// The bin stays clickable; a failed decode just adds nothing.
					return false;
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
		for (const id of selectedSegmentIds) {
			const segment = edit.segments.find((s) => s.id === id);
			if (segment) engine.applySegmentEffects(segment.id, segment.effects);
		}
	}, [edit.segments, engine, selectedSegmentIds]);

	useEffect(() => {
		const structureKey = playbackStructureKey(edit);
		const structureChanged = playbackStructureRef.current !== structureKey;
		playbackStructureRef.current = structureKey;
		if (structureChanged && !draggingRef.current && engine.isPlaying) {
			if (playbackRefreshRef.current !== null) {
				clearTimeout(playbackRefreshRef.current);
			}
			// Structural edits, including track mute changes, require a new source
			// schedule. Streaming effect edits are applied live by the AudioWorklet.
			playbackRefreshRef.current = window.setTimeout(() => {
				playbackRefreshRef.current = null;
				if (!engine.isPlaying) return;
				engine.play(
					editRef.current,
					positionRef.current,
					buffersRef.current,
					loopRef.current,
				);
			}, 120);
		}
		return () => {
			if (playbackRefreshRef.current !== null) {
				clearTimeout(playbackRefreshRef.current);
				playbackRefreshRef.current = null;
			}
		};
	}, [edit, engine]);

	const select = useCallback((id: string | null) => {
		if (id === null) {
			setSelectedSegmentIds([]);
			return;
		}
		setSelectedSegmentIds(expandMergeGroups(editRef.current.segments, [id]));
	}, []);

	/** Replaces the selection with the given ids (marquee multi-select). */
	const selectMany = useCallback((ids: string[]) => {
		setSelectedSegmentIds(expandMergeGroups(editRef.current.segments, ids));
	}, []);

	/**
	 * Ctrl/Cmd-click toggle: adds an unselected segment to the selection or
	 * removes a selected one, leaving the rest untouched. Returns the next
	 * selection so the caller can act on it synchronously. Merged units
	 * toggle as a whole.
	 */
	const toggleSelect = useCallback(
		(id: string) => {
			const ids = expandMergeGroups(editRef.current.segments, [id]);
			const next = ids.every((selected) =>
				selectedSegmentIds.includes(selected),
			)
				? selectedSegmentIds.filter((selected) => !ids.includes(selected))
				: [...selectedSegmentIds, ...ids];
			setSelectedSegmentIds(Array.from(new Set(next)));
			return next;
		},
		[selectedSegmentIds],
	);

	const beginGesture = useCallback(() => {
		draggingRef.current = true;
	}, []);

	const endGesture = useCallback(() => {
		draggingRef.current = false;
		flush();
	}, [flush]);

	const zoomAt = useCallback(
		(factor: number, anchorSec: number) => {
			const nextWidth = Math.max(1, Math.min(120, viewWidthSec * factor));
			const anchorFraction =
				(anchorSec - viewStartSec) / Math.max(1, viewWidthSec);
			const nextStart = anchorSec - anchorFraction * nextWidth;
			const nextMaxStart = Math.max(0, contentDurationSec - nextWidth);
			setViewStartSec(Math.min(nextMaxStart, Math.max(0, nextStart)));
			setViewWidthSec(nextWidth);
		},
		[contentDurationSec, viewStartSec, viewWidthSec],
	);

	const zoom = useCallback(
		(factor: number) => zoomAt(factor, positionRef.current),
		[zoomAt],
	);

	const setViewStart = useCallback(
		(startSec: number) => {
			if (!Number.isFinite(startSec)) return;
			setViewStartSec(Math.min(viewMaxStartSec, Math.max(0, startSec)));
		},
		[viewMaxStartSec],
	);

	const panView = useCallback(
		(deltaSec: number) => {
			if (!Number.isFinite(deltaSec) || deltaSec === 0) return;
			setViewStartSec((current) =>
				Math.min(viewMaxStartSec, Math.max(0, current + deltaSec)),
			);
		},
		[viewMaxStartSec],
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
		if (selectedSegmentIds.length === 0) return;
		if (isInspectorFeatureDisabled("delete", selectedSegmentIds.length)) return;
		apply((current) => ({
			...current,
			segments: current.segments.filter(
				(s) => !selectedSegmentIds.includes(s.id),
			),
		}));
		setSelectedSegmentIds([]);
	}, [apply, selectedSegmentIds]);

	const splitSelectedAtPlayhead = useCallback(() => {
		if (isInspectorFeatureDisabled("split", selectedSegmentIds.length)) return;
		if (selectedSegmentIds.length !== 1) return;
		const id = selectedSegmentIds[0];
		if (!id) return;
		apply((current) => splitSegment(current, id, positionRef.current));
	}, [apply, selectedSegmentIds]);

	/**
	 * Merges the selected segments into one unit when they form a snapped
	 * chain on a single track. The segments keep their own sources and
	 * effects - playback and export stay unchanged - but they render,
	 * select and move as one element. Refused merges surface the reason in
	 * `mergeWarning` instead of changing the edit.
	 */
	const mergeSelected = useCallback(() => {
		const segments = edit.segments.filter((segment) =>
			selectedSegmentIds.includes(segment.id),
		);
		const reason = mergeBlockReason(segments);
		if (reason === "already-merged") return;
		if (reason !== null) {
			setMergeWarning(MERGE_BLOCK_MESSAGES[reason]);
			return;
		}
		const result = mergeSegments(edit, selectedSegmentIds);
		if (!result) return;
		apply(() => result.edit);
	}, [apply, edit, selectedSegmentIds]);

	/** Breaks the selected merged unit back into individual segments. */
	const unmergeSelected = useCallback(() => {
		if (selectedSegmentIds.length === 0) return;
		apply((current) => unmergeSegments(current, selectedSegmentIds));
	}, [apply, selectedSegmentIds]);

	const dismissMergeWarning = useCallback(() => setMergeWarning(null), []);

	const toggleReverse = useCallback(() => {
		if (selectedSegmentIds.length === 0) return;
		if (isInspectorFeatureDisabled("reverse", selectedSegmentIds.length))
			return;
		const ids = selectedSegmentIds;
		apply((current) => ({
			...current,
			segments: current.segments.map((segment) =>
				ids.includes(segment.id)
					? {
							...segment,
							effects: {
								...segment.effects,
								reverse: !segment.effects.reverse,
							},
						}
					: segment,
			),
		}));
	}, [apply, selectedSegmentIds]);

	const selectedSegments = edit.segments.filter((s) =>
		selectedSegmentIds.includes(s.id),
	);
	const selectedSegment = selectedSegments[0] ?? null;
	const multiSelected = selectedSegments.length > 1;

	const selectTrack = useCallback(
		(track: number) => setActiveTrack(Math.max(0, track)),
		[],
	);

	const copy = useCallback(() => {
		const copied = cloneTimelineSegments(
			segmentsForCopy(edit.segments, selectedSegmentIds, copyAllSelected),
		);
		if (copied.length === 0) return;
		setClipboard(copied);
		setCopySourceIds(copied.map((segment) => segment.id));
	}, [copyAllSelected, edit.segments, selectedSegmentIds]);

	const setPasteTarget = useCallback((target: PasteTarget | null) => {
		pasteTargetRef.current = target;
	}, []);

	const cut = useCallback(() => {
		if (selectedSegments.length === 0) return;
		const copied = cloneTimelineSegments(selectedSegments);
		setClipboard(copied);
		setCopySourceIds([]);
		removeSelected();
	}, [removeSelected, selectedSegments]);

	// Pasting prefers a legal empty space under the mouse. Otherwise it falls
	// back to the active track at the playhead, preserving the copied layout and
	// snapping the complete multi-track group past every obstruction.
	const paste = useCallback(() => {
		if (!clipboard || clipboard.length === 0) return;
		const sourceStart = Math.min(
			...clipboard.map((segment) => segment.timelineStart),
		);
		const sourceTrack = Math.min(...clipboard.map((segment) => segment.track));
		const mouseTarget = pasteTargetRef.current;
		const useMouseTarget = Boolean(
			mouseTarget &&
				isPastePlacementLegal(
					clipboard,
					editRef.current.segments,
					mouseTarget.startSec,
					mouseTarget.track,
				),
		);
		const rawStart = positionRef.current;
		const targetTrack = useMouseTarget
			? (mouseTarget?.track ?? activeTrack)
			: activeTrack;
		const start = useMouseTarget
			? (mouseTarget?.startSec ?? rawStart)
			: snapToPastedLayout(
					rawStart,
					clipboard,
					editRef.current.segments,
					activeTrack,
				);
		const pasted = duplicateTimelineSegments(
			clipboard,
			start - sourceStart,
			targetTrack - sourceTrack,
		);
		const pastedIds = pasted.map((segment) => segment.id);
		const highestPastedTrack = Math.max(
			...pasted.map((segment) => segment.track),
		);
		apply((current) => {
			const tracks = Math.max(current.tracks, highestPastedTrack + 1);
			const mutedTracks =
				current.mutedTracks.length >= tracks
					? current.mutedTracks
					: [
							...current.mutedTracks,
							...Array.from(
								{ length: tracks - current.mutedTracks.length },
								() => false,
							),
						];
			return {
				...current,
				segments: [...current.segments, ...pasted],
				tracks,
				mutedTracks,
			};
		});
		setCopySourceIds([]);
		// apply() schedules the edit update, so select the known new ids
		// directly instead of asking the still-current edit to expand them.
		setSelectedSegmentIds(pastedIds);
	}, [activeTrack, apply, clipboard]);

	const sourceDuration = useCallback((sourceId: string): number | null => {
		return buffersRef.current.get(sourceId)?.duration ?? null;
	}, []);

	const preloadSources = useCallback(
		async (guildId: string, sourceIds: string[]) => {
			const missing = sourceIds.filter(
				(sourceId) => !buffersRef.current.has(sourceId),
			);
			if (missing.length === 0) return;
			setLoadingClips((previous) => {
				const next = new Map(previous);
				for (const sourceId of missing) next.set(sourceId, true);
				return next;
			});
			await Promise.all(
				missing.map((sourceId) =>
					loadClipBuffer(guildId, sourceId)
						.then((buffer) => registerBuffer(sourceId, buffer))
						.catch(() => {
							// Segments referencing an unreadable source stay silent;
							// the edit itself remains intact for re-export.
						})
						.finally(() => {
							setLoadingClips((previous) => {
								const next = new Map(previous);
								next.delete(sourceId);
								return next;
							});
						}),
				),
			);
		},
		[registerBuffer],
	);

	return {
		edit,
		preview,
		flush,
		apply,
		undo,
		redo,
		reset,
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
		selectedSegmentId: selectedSegmentIds[0] ?? null,
		selectedSegmentIds,
		selectedSegments,
		selectedSegment,
		multiSelected,
		select,
		selectMany,
		toggleSelect,
		activeTrack,
		selectTrack,
		copySourceId: copySourceIds[0] ?? null,
		copySourceIds,
		copy,
		setPasteTarget,
		cut,
		paste,
		loadClip,
		registerBuffer,
		sourceDuration,
		preloadSources,
		loadingClips,
		viewStartSec,
		viewWidthSec,
		timelineDurationSec,
		viewMaxStartSec,
		zoom,
		zoomAt,
		setViewStart,
		panView,
		fitView,
		beginGesture,
		endGesture,
		removeSelected,
		removeTrack,
		splitSelectedAtPlayhead,
		mergeSelected,
		unmergeSelected,
		mergeWarning,
		dismissMergeWarning,
		toggleTrackMute,
		toggleReverse,
	};
}

function playbackStructureKey(edit: ClipEdit): string {
	return [
		edit.tracks,
		edit.mutedTracks.map((muted) => (muted ? 1 : 0)).join(","),
		...edit.segments.map((segment) =>
			[
				segment.id,
				segment.source,
				segment.sourceId,
				segment.timelineStart,
				sharedDspPreprocessKey(segment),
			].join(":"),
		),
	].join("|");
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
