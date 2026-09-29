import { type RefObject, useEffect, useRef, useState } from "react";
import type { ClipEditorEngine } from "./engine";
import type { ClipEdit } from "./model";
import { sharedDspPreprocessKey, warmSharedDsp } from "./sharedDsp";

/**
 * The preview transport: play, pause, seek, and loop, plus the playhead the
 * engine reports each animation frame. A structural edit reschedules playback;
 * effect edits reach the engine live without one.
 */
export function usePlaybackTransport(options: {
	engine: ClipEditorEngine;
	edit: ClipEdit;
	/** Every decoded source the edit can reference, keyed by source id. */
	buffersRef: RefObject<Map<string, AudioBuffer>>;
	/** True during a drag gesture, which reschedules once it ends instead. */
	draggingRef: RefObject<boolean>;
}) {
	const { engine, edit, buffersRef, draggingRef } = options;
	const [positionSec, setPositionSec] = useState(0);
	const [playing, setPlaying] = useState(false);
	const [loop, setLoop] = useState(false);
	/**
	 * True when preview playback is running without the shared DSP, so pitch,
	 * speed, and reverse are not applied. Exports are unaffected.
	 */
	const [effectsUnavailable, setEffectsUnavailable] = useState(false);
	/** The playhead between renders, for callbacks and the frame loop. */
	const positionRef = useRef(0);
	const loopRef = useRef(loop);
	const frameRef = useRef<number | null>(null);
	const playbackStructureRef = useRef(playbackStructureKey(edit));

	useEffect(() => {
		loopRef.current = loop;
	}, [loop]);

	useEffect(() => {
		void warmSharedDsp();
		return () => engine.dispose();
	}, [engine]);

	// Follow the engine's playhead while playing. The loop ends itself when
	// playback finishes; pausing cancels it at once (see togglePlay), so a frame
	// already queued cannot mistake the pause for the end of playback.
	useEffect(() => {
		if (!playing) return;
		function followPlayhead() {
			if (!engine.isPlaying) {
				frameRef.current = null;
				setPlaying(false);
				positionRef.current = 0;
				setPositionSec(0);
				return;
			}
			// Preparation runs while playback starts, so the fallback only becomes
			// known after the first frames; keep the notice in sync from here.
			const fallback = engine.isUsingNativeEffectsFallback;
			setEffectsUnavailable((current) =>
				current === fallback ? current : fallback,
			);
			positionRef.current = engine.positionSec;
			setPositionSec(engine.positionSec);
			frameRef.current = requestAnimationFrame(followPlayhead);
		}
		frameRef.current = requestAnimationFrame(followPlayhead);
		return () => {
			if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
			frameRef.current = null;
		};
	}, [engine, playing]);

	useEffect(() => {
		const structureKey = playbackStructureKey(edit);
		const structureChanged = playbackStructureRef.current !== structureKey;
		playbackStructureRef.current = structureKey;
		if (!structureChanged || draggingRef.current || !engine.isPlaying) return;
		// Structural edits, including track mute changes, require a new source
		// schedule. Streaming effect edits are applied live by the AudioWorklet.
		const refresh = window.setTimeout(() => {
			if (!engine.isPlaying) return;
			engine.play(
				edit,
				positionRef.current,
				buffersRef.current,
				loopRef.current,
			);
		}, 120);
		return () => clearTimeout(refresh);
	}, [buffersRef, draggingRef, edit, engine]);

	const togglePlay = () => {
		if (engine.isPlaying) {
			engine.pause();
			if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
			frameRef.current = null;
			setPlaying(false);
			positionRef.current = engine.positionSec;
			setPositionSec(engine.positionSec);
			return;
		}
		setPlaying(true);
		engine.play(edit, positionRef.current, buffersRef.current, loopRef.current);
	};

	const seek = (sec: number) => {
		const clamped = Math.max(0, sec);
		if (engine.isPlaying) {
			engine.seekTo(edit, clamped, buffersRef.current, loopRef.current);
		}
		positionRef.current = clamped;
		setPositionSec(clamped);
	};

	const setLooping = (next: boolean) => {
		setLoop(next);
		loopRef.current = next;
		if (engine.isPlaying) {
			engine.seekTo(edit, positionRef.current, buffersRef.current, next);
		}
	};

	return {
		positionSec,
		positionRef,
		playing,
		togglePlay,
		seek,
		loop,
		setLooping,
		effectsUnavailable,
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
