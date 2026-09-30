import { useEffect, useRef, useState } from "react";
import { ClipEditorEngine } from "./engine";
import { isInspectorFeatureDisabled } from "./inspectorFeaturePolicy";
import {
	type ClipEdit,
	cloneTimelineSegments,
	duplicateTimelineSegments,
	emptyEdit,
	expandMergeGroups,
	isPastePlacementLegal,
	MERGE_BLOCK_MESSAGES,
	makeSegment,
	mergeBlockReason,
	mergeSegments,
	newSegmentId,
	removeTrack as removeTrackFromEdit,
	segmentDuration,
	segmentsForCopy,
	snapToPastedLayout,
	splitSegment,
	type TimelineSegment,
	toggleTrackMute as toggleTrackMuteInEdit,
	unmergeSegments,
} from "./model";
import { useEditHistory } from "./useEditHistory";
import { usePlaybackTransport } from "./usePlaybackTransport";
import { useSourceBuffers } from "./useSourceBuffers";
import { useTimelineViewport } from "./useTimelineViewport";

export type UseClipEditorReturn = ReturnType<typeof useClipEditor>;

export interface PasteTarget {
	startSec: number;
	track: number;
}

/**
 * The clip editor's state and actions: the undoable edit, the selection and
 * clipboard, and the edit operations. Playback, source buffers, and the
 * timeline viewport live in their own hooks, composed here.
 */
export function useClipEditor(
	options: { copyAllSelected?: boolean; initialEdit?: ClipEdit } = {},
) {
	const copyAllSelected = options.copyAllSelected ?? true;
	const history = useEditHistory(options.initialEdit ?? emptyEdit());
	const {
		edit,
		committed,
		preview,
		flush,
		apply,
		undo,
		redo,
		canUndo,
		canRedo,
		reset,
	} = history;
	const [engine] = useState(() => new ClipEditorEngine());
	const draggingRef = useRef(false);
	const sources = useSourceBuffers();
	const transport = usePlaybackTransport({
		engine,
		edit,
		buffersRef: sources.buffersRef,
		draggingRef,
	});
	const { positionRef } = transport;
	const viewport = useTimelineViewport(edit, positionRef);

	const [selectedSegmentIds, setSelectedSegmentIds] = useState<string[]>([]);
	/** Track that clicks on the timeline activate; paste and bin adds land here. */
	const [activeTrack, setActiveTrack] = useState(0);
	const [clipboard, setClipboard] = useState<TimelineSegment[] | null>(null);
	/** Segments whose data is in the clipboard; they show the copied ring. */
	const [copySourceIds, setCopySourceIds] = useState<string[]>([]);
	/** Warning describing why the last merge attempt was refused. */
	const [mergeWarning, setMergeWarning] = useState<string | null>(null);
	const pasteTargetRef = useRef<PasteTarget | null>(null);

	useEffect(() => {
		for (const id of selectedSegmentIds) {
			const segment = edit.segments.find((s) => s.id === id);
			if (segment) engine.applySegmentEffects(segment.id, segment.effects);
		}
	}, [edit.segments, engine, selectedSegmentIds]);

	const setMasterVolume = (db: number) => {
		apply((current) => ({ ...current, masterVolumeDb: db }));
		engine.setMasterVolume(db);
	};

	const toggleTrackMute = (track: number) => {
		apply((current) => toggleTrackMuteInEdit(current, track));
	};

	const removeTrack = (track: number) => {
		if (edit.tracks <= 1 || track < 0 || track >= edit.tracks) return;
		const removedIds = new Set(
			edit.segments
				.filter((segment) => segment.track === track)
				.map((segment) => segment.id),
		);
		const lastRemainingTrack = edit.tracks - 2;
		apply((current) => removeTrackFromEdit(current, track));
		setSelectedSegmentIds((ids) => ids.filter((id) => !removedIds.has(id)));
		setActiveTrack((active) =>
			active > track ? active - 1 : Math.min(active, lastRemainingTrack),
		);
	};

	const loadClip = (
		guildId: string,
		clipId: string,
		lengthSec: number,
		track: number,
		timelineStart?: number,
	) => {
		// Allocate identity once for the action, outside replayable updaters.
		const segment = makeSegment("clip", clipId, 0, lengthSec, 0, track);
		// A failed decode adds nothing; the bin stays clickable.
		return sources.withSource(guildId, clipId, () =>
			apply((current) =>
				addSegmentAt(
					current,
					segment,
					timelineStart ?? endOfTrack(current, track),
				),
			),
		);
	};

	const select = (id: string | null) => {
		setSelectedSegmentIds(
			id === null ? [] : expandMergeGroups(edit.segments, [id]),
		);
	};

	/** Replaces the selection with the given ids (marquee multi-select). */
	const selectMany = (ids: string[]) => {
		setSelectedSegmentIds(expandMergeGroups(edit.segments, ids));
	};

	/**
	 * Ctrl/Cmd-click toggle: adds an unselected segment to the selection or
	 * removes a selected one, leaving the rest untouched. Returns the next
	 * selection so the caller can act on it synchronously. Merged units
	 * toggle as a whole.
	 */
	const toggleSelect = (id: string) => {
		const ids = expandMergeGroups(edit.segments, [id]);
		const next = ids.every((selected) => selectedSegmentIds.includes(selected))
			? selectedSegmentIds.filter((selected) => !ids.includes(selected))
			: [...selectedSegmentIds, ...ids];
		setSelectedSegmentIds(Array.from(new Set(next)));
		return next;
	};

	const beginGesture = () => {
		draggingRef.current = true;
	};

	const endGesture = () => {
		draggingRef.current = false;
		flush();
	};

	const removeSelected = () => {
		if (selectedSegmentIds.length === 0) return;
		if (isInspectorFeatureDisabled("delete", selectedSegmentIds.length)) return;
		apply((current) => ({
			...current,
			segments: current.segments.filter(
				(s) => !selectedSegmentIds.includes(s.id),
			),
		}));
		setSelectedSegmentIds([]);
	};

	const splitSelectedAtPlayhead = () => {
		if (isInspectorFeatureDisabled("split", selectedSegmentIds.length)) return;
		if (selectedSegmentIds.length !== 1) return;
		const id = selectedSegmentIds[0];
		if (!id) return;
		const atSec = positionRef.current;
		const secondId = newSegmentId();
		apply((current) => splitSegment(current, id, atSec, secondId));
	};

	/**
	 * Merges the selected segments into one unit when they form a snapped
	 * chain on a single track. The segments keep their own sources and
	 * effects - playback and export stay unchanged - but they render,
	 * select and move as one element. Refused merges surface the reason in
	 * `mergeWarning` instead of changing the edit.
	 */
	const mergeSelected = () => {
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
	};

	/** Breaks the selected merged unit back into individual segments. */
	const unmergeSelected = () => {
		if (selectedSegmentIds.length === 0) return;
		apply((current) => unmergeSegments(current, selectedSegmentIds));
	};

	const dismissMergeWarning = () => setMergeWarning(null);

	const toggleReverse = () => {
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
	};

	const selectedSegments = edit.segments.filter((s) =>
		selectedSegmentIds.includes(s.id),
	);
	const selectedSegment = selectedSegments[0] ?? null;
	const multiSelected = selectedSegments.length > 1;

	const selectTrack = (track: number) => setActiveTrack(Math.max(0, track));

	const copy = () => {
		const copied = cloneTimelineSegments(
			segmentsForCopy(edit.segments, selectedSegmentIds, copyAllSelected),
		);
		if (copied.length === 0) return;
		setClipboard(copied);
		setCopySourceIds(copied.map((segment) => segment.id));
	};

	const setPasteTarget = (target: PasteTarget | null) => {
		pasteTargetRef.current = target;
	};

	const cut = () => {
		if (selectedSegments.length === 0) return;
		const copied = cloneTimelineSegments(selectedSegments);
		setClipboard(copied);
		setCopySourceIds([]);
		removeSelected();
	};

	// Pasting prefers a legal empty space under the mouse. Otherwise it falls
	// back to the active track at the playhead, preserving the copied layout and
	// snapping the complete multi-track group past every obstruction.
	const paste = () => {
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
					edit.segments,
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
			: snapToPastedLayout(rawStart, clipboard, edit.segments, activeTrack);
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
	};

	return {
		edit,
		committed,
		preview,
		flush,
		apply,
		undo,
		redo,
		reset,
		canUndo,
		canRedo,
		positionSec: transport.positionSec,
		setPosition: transport.seek,
		playing: transport.playing,
		togglePlay: transport.togglePlay,
		loop: transport.loop,
		setLooping: transport.setLooping,
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
		registerBuffer: sources.registerBuffer,
		sourceDuration: sources.sourceDuration,
		preloadSources: sources.preloadSources,
		loadingClips: sources.loadingClips,
		viewStartSec: viewport.viewStartSec,
		viewWidthSec: viewport.viewWidthSec,
		timelineDurationSec: viewport.timelineDurationSec,
		viewMaxStartSec: viewport.viewMaxStartSec,
		zoom: viewport.zoom,
		zoomAt: viewport.zoomAt,
		setViewStart: viewport.setViewStart,
		panView: viewport.panView,
		fitView: viewport.fitView,
		beginGesture,
		endGesture,
		removeSelected,
		removeTrack,
		splitSelectedAtPlayhead,
		mergeSelected,
		unmergeSelected,
		mergeWarning,
		dismissMergeWarning,
		effectsUnavailable: transport.effectsUnavailable,
		toggleTrackMute,
		toggleReverse,
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
	segment: TimelineSegment,
	timelineStart: number,
): ClipEdit {
	return {
		...edit,
		segments: [...edit.segments, { ...segment, timelineStart }],
		tracks: Math.max(edit.tracks, segment.track + 1),
	};
}
