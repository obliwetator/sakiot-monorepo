import { type MutableRefObject, useCallback, useRef, useState } from "react";
import type { SessionSelection } from "./logicalSessionSelection";
import { selectionContainsPosition } from "./logicalSessionSelection";

export interface PlaybackBound {
	stopMs: number;
	loopToMs: number | null;
}

interface PlaybackBoundOptions {
	startAtRef: MutableRefObject<(position: number, autoplay: boolean) => void>;
	positionRef: MutableRefObject<number>;
	playingRef: MutableRefObject<boolean>;
	durationRef: MutableRefObject<number>;
	onLoopDisabledRef: MutableRefObject<() => void>;
	/** The owning hook's stop routine, run when bound playback must end first. */
	stopRef: MutableRefObject<() => void>;
	/** Clears the owning hook's playback error before a bound starts. */
	onBeforePlayRef: MutableRefObject<() => void>;
	setSeekPreviewMs: (value: number | null) => void;
}

/**
 * The loop/stop boundary both playback hooks share: which logical position the
 * current selection stops at or loops back to, plus the callbacks that arm,
 * apply, and re-sync it. Segmented and silence-free playback had byte-identical
 * copies of this, which is exactly the kind of state machine that drifts.
 */
export function usePlaybackBound(options: PlaybackBoundOptions) {
	const {
		startAtRef,
		positionRef,
		playingRef,
		durationRef,
		onLoopDisabledRef,
		stopRef,
		onBeforePlayRef,
		setSeekPreviewMs,
	} = options;
	const [boundActive, setBoundActive] = useState(false);
	const boundRef = useRef<PlaybackBound | null>(null);

	const clearBound = useCallback(() => {
		boundRef.current = null;
		setBoundActive(false);
	}, []);

	const applyBound = useCallback(
		(atMs: number): boolean => {
			const bound = boundRef.current;
			if (!bound || atMs < bound.stopMs) return false;
			if (bound.loopToMs !== null) {
				startAtRef.current(bound.loopToMs, true);
				return true;
			}
			boundRef.current = null;
			setBoundActive(false);
			startAtRef.current(bound.stopMs, false);
			return true;
		},
		[startAtRef],
	);

	const togglePlay = useCallback(
		(selection: SessionSelection, loop: boolean) => {
			if (playingRef.current) {
				stopRef.current();
				return;
			}
			onBeforePlayRef.current();
			if (loop && selection[1] > selection[0]) {
				boundRef.current = { stopMs: selection[1], loopToMs: selection[0] };
				setBoundActive(true);
				const current = positionRef.current;
				startAtRef.current(
					selectionContainsPosition(selection, current)
						? current
						: selection[0],
					true,
				);
				return;
			}
			clearBound();
			startAtRef.current(
				positionRef.current >= durationRef.current ? 0 : positionRef.current,
				true,
			);
		},
		[
			clearBound,
			durationRef,
			onBeforePlayRef,
			playingRef,
			positionRef,
			startAtRef,
			stopRef,
		],
	);

	const togglePreview = useCallback(
		(selection: SessionSelection, loop: boolean) => {
			if (boundRef.current) {
				stopRef.current();
				return;
			}
			if (selection[1] <= selection[0]) return;
			boundRef.current = {
				stopMs: selection[1],
				loopToMs: loop ? selection[0] : null,
			};
			setBoundActive(true);
			setSeekPreviewMs(null);
			startAtRef.current(selection[0], true);
		},
		[setSeekPreviewMs, startAtRef, stopRef],
	);

	const updateLoop = useCallback(
		(enabled: boolean, selection: SessionSelection) => {
			if (!enabled) {
				if (boundRef.current) boundRef.current.loopToMs = null;
				return;
			}
			if (boundRef.current) {
				boundRef.current = { stopMs: selection[1], loopToMs: selection[0] };
				setBoundActive(true);
				startAtRef.current(selection[0], playingRef.current);
				return;
			}
			clearBound();
			startAtRef.current(selection[0], false);
		},
		[clearBound, playingRef, startAtRef],
	);

	const syncBound = useCallback(
		(selection: SessionSelection, loop: boolean) => {
			if (!boundRef.current) return;
			if (!selectionContainsPosition(selection, positionRef.current)) {
				clearBound();
				if (loop) onLoopDisabledRef.current();
				return;
			}
			boundRef.current = {
				stopMs: selection[1],
				loopToMs: loop ? selection[0] : null,
			};
			applyBound(positionRef.current);
		},
		[applyBound, clearBound, onLoopDisabledRef, positionRef],
	);

	/** Re-arms the bound for a seek target; the caller moves the media. */
	const prepareSeek = useCallback(
		(selection: SessionSelection, loop: boolean, targetMs: number) => {
			if (loop && !selectionContainsPosition(selection, targetMs)) {
				clearBound();
				onLoopDisabledRef.current();
			} else if (boundRef.current) {
				boundRef.current = {
					stopMs: selection[1],
					loopToMs: loop ? selection[0] : null,
				};
			}
		},
		[clearBound, onLoopDisabledRef],
	);

	return {
		boundActive,
		clearBound,
		applyBound,
		togglePlay,
		togglePreview,
		updateLoop,
		syncBound,
		prepareSeek,
	};
}
