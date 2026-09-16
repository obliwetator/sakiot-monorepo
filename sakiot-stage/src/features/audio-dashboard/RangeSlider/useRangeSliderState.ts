import type { Dispatch, RefObject, SetStateAction } from "react";
import { useCallback, useEffect, useState } from "react";
import {
	playbackShortcutTargetAcceptsText,
	playbackShortcutTargetOwnsArrows,
} from "../playbackShortcuts";

const ArrowKeySkip = 5;
const CtrlArrowKeySkip = 30;
const MinDistance = 1;

export interface RangeSliderState {
	playing: boolean;
	startEnd: number[];
	setStartEnd: Dispatch<SetStateAction<number[]>>;
	durationSec: number;
	handleChange: (values: number[]) => void;
	togglePlay: () => void;
	pinEnd: () => void;
}

export function useRangeSliderState(args: {
	audioRef: HTMLAudioElement;
	intervalRef: RefObject<number | undefined>;
	trueDuration?: number | null;
	liveStartedAt?: number | null;
}): RangeSliderState {
	const initialDuration = args.liveStartedAt
		? Math.max(0, (Date.now() - args.liveStartedAt) / 1000)
		: args.trueDuration && Number.isFinite(args.trueDuration)
			? args.trueDuration
			: Number.isFinite(args.audioRef.duration)
				? args.audioRef.duration
				: 0;
	const [actualDuration, setActualDuration] = useState(initialDuration);
	const [playing, setPlaying] = useState(false);
	const [startEnd, setStartEnd] = useState<number[]>([
		args.audioRef.currentTime || 0,
		actualDuration,
	]);
	const [rightPinned, setRightPinned] = useState(false);

	useEffect(() => {
		if (args.liveStartedAt) return;
		if (args.trueDuration && Number.isFinite(args.trueDuration)) {
			const nextDuration = args.trueDuration as number;
			setActualDuration(nextDuration);
			if (nextDuration < MinDistance) {
				if (!rightPinned) {
					setStartEnd(() => [0, nextDuration]);
				} else {
					setStartEnd((prev) => [0, Math.min(prev[1], nextDuration)]);
				}
				return;
			}
			if (!rightPinned) {
				setStartEnd((prev) => [
					Math.min(prev[0], Math.max(0, nextDuration - MinDistance)),
					nextDuration,
				]);
			} else {
				setStartEnd((prev) => [
					Math.min(prev[0], Math.max(0, nextDuration - MinDistance)),
					Math.min(prev[1], nextDuration),
				]);
			}
		}
	}, [args.liveStartedAt, args.trueDuration, rightPinned]);

	useEffect(() => {
		if (!args.liveStartedAt) return;
		const startedAt = args.liveStartedAt;
		const tick = () => {
			const dur = (Date.now() - startedAt) / 1000;
			if (dur <= 0) return;
			setActualDuration(dur);
			setStartEnd((prev) => {
				const end = rightPinned ? Math.min(prev[1], dur) : dur;
				const start = Math.min(prev[0], Math.max(0, end - MinDistance));
				return [start, end];
			});
		};
		tick();
		const id = window.setInterval(tick, 250);
		return () => window.clearInterval(id);
	}, [args.liveStartedAt, rightPinned]);

	const stopPlayback = useCallback(() => {
		clearInterval(args.intervalRef.current);
		args.intervalRef.current = undefined;
		args.audioRef.pause();
	}, [args.audioRef, args.intervalRef]);

	const end = startEnd[1];
	const advancePlayhead = useCallback(() => {
		const time = args.audioRef.currentTime;
		if (time >= end) stopPlayback();
		const next =
			time >= end
				? Math.max(0, end - MinDistance)
				: Math.min(time, actualDuration, end);
		setStartEnd((previous) => [next, previous[1]]);
	}, [actualDuration, args.audioRef, end, stopPlayback]);

	useEffect(() => {
		const audio = args.audioRef;
		const onPlay = () => setPlaying(true);
		const onPause = () => setPlaying(false);
		audio.addEventListener("play", onPlay);
		audio.addEventListener("pause", onPause);
		audio.addEventListener("timeupdate", advancePlayhead);
		return () => {
			audio.removeEventListener("play", onPlay);
			audio.removeEventListener("pause", onPause);
			audio.removeEventListener("timeupdate", advancePlayhead);
		};
	}, [args.audioRef, advancePlayhead]);

	useEffect(() => {
		const handleArrowKeys = (event: KeyboardEvent) => {
			if (playbackShortcutTargetOwnsArrows(event.target)) return;
			if (event.key === "ArrowRight") {
				event.preventDefault();
				const skip =
					event.ctrlKey || event.metaKey ? CtrlArrowKeySkip : ArrowKeySkip;
				const next = Math.max(
					0,
					Math.min(startEnd[0] + skip, actualDuration, end - MinDistance),
				);
				args.audioRef.currentTime = next;
				setStartEnd([next, end]);
			} else if (event.key === "ArrowLeft") {
				event.preventDefault();
				const skip =
					event.ctrlKey || event.metaKey ? CtrlArrowKeySkip : ArrowKeySkip;
				const next = Math.max(startEnd[0] - skip, 0);
				args.audioRef.currentTime = next;
				setStartEnd([next, end]);
			} else {
				return;
			}
		};
		window.addEventListener("keydown", handleArrowKeys);
		return () => window.removeEventListener("keydown", handleArrowKeys);
	}, [actualDuration, args.audioRef, end, startEnd]);

	// The interval is parent-owned (AudioInterface), so nothing else clears it
	// when this component unmounts mid-playback; without this cleanup it would
	// keep firing forever on a detached audio element.
	useEffect(() => {
		const intervalRef = args.intervalRef;
		if (playing) {
			intervalRef.current = window.setInterval(advancePlayhead, 1000);
		}
		return () => {
			clearInterval(intervalRef.current);
			intervalRef.current = undefined;
		};
	}, [advancePlayhead, args.intervalRef, playing]);

	const togglePlay = useCallback(() => {
		// React may replay state updaters; media commands belong to the event.
		if (playing) {
			stopPlayback();
			setPlaying(false);
			return;
		}
		setPlaying(true);
		void args.audioRef.play().catch(() => setPlaying(false));
	}, [args.audioRef, playing, stopPlayback]);

	useEffect(() => {
		const handleSpace = (event: KeyboardEvent) => {
			if (event.key !== " " && event.code !== "Space") return;
			if (playbackShortcutTargetAcceptsText(event.target)) return;
			if (event.repeat) return;
			event.preventDefault();
			togglePlay();
		};
		window.addEventListener("keydown", handleSpace);
		return () => window.removeEventListener("keydown", handleSpace);
	}, [togglePlay]);

	const handleChange = (newValue: number[]) => {
		if (newValue[0] !== startEnd[0]) {
			const newStart = Math.max(
				0,
				Math.min(newValue[0], startEnd[1] - MinDistance),
			);
			args.audioRef.currentTime = newStart;
			setStartEnd([newStart, startEnd[1]]);
		} else {
			const newEnd = Math.min(
				actualDuration,
				Math.max(newValue[1], startEnd[0] + MinDistance),
			);
			const newStart = Math.min(startEnd[0], Math.max(0, newEnd - MinDistance));
			setStartEnd([newStart, newEnd]);
			setRightPinned(true);
		}
	};

	return {
		playing,
		startEnd,
		setStartEnd,
		durationSec: actualDuration,
		handleChange,
		togglePlay,
		pinEnd: () => setRightPinned(true),
	};
}
