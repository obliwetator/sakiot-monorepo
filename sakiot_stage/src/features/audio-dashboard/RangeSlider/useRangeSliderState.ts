import type { Dispatch, RefObject, SetStateAction } from "react";
import { useCallback, useEffect, useState } from "react";

const ArrowKeySkip = 5;
const CtrlArrowKeySkip = 30;
const MinDistance = 1;

export interface RangeSliderState {
	playing: boolean;
	startEnd: number[];
	setStartEnd: Dispatch<SetStateAction<number[]>>;
	durationSec: number;
	handleChange: (
		event: Event,
		newValue: number | number[],
		activeThumb: number,
	) => void;
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
			setActualDuration(args.trueDuration);
			if (!rightPinned) {
				setStartEnd((prev) => [
					prev[0],
					Math.max(prev[1], args.trueDuration as number),
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

	useEffect(() => {
		const audio = args.audioRef;
		const onPlay = () => setPlaying(true);
		const onPause = () => setPlaying(false);
		const onTime = () => {
			setStartEnd((prev) => [
				Math.min(audio.currentTime, actualDuration),
				prev[1],
			]);
		};
		audio.addEventListener("play", onPlay);
		audio.addEventListener("pause", onPause);
		audio.addEventListener("timeupdate", onTime);
		return () => {
			audio.removeEventListener("play", onPlay);
			audio.removeEventListener("pause", onPause);
			audio.removeEventListener("timeupdate", onTime);
		};
	}, [actualDuration, args.audioRef]);

	useEffect(() => {
		const handleArrowKeys = (event: KeyboardEvent) => {
			if (event.key === "ArrowRight") {
				const skip = event.ctrlKey ? CtrlArrowKeySkip : ArrowKeySkip;
				setStartEnd((s) => {
					const next = Math.min(
						s[0] + skip,
						actualDuration,
						s[1] - MinDistance,
					);
					args.audioRef.currentTime = next;
					return [next, s[1]];
				});
			} else if (event.key === "ArrowLeft") {
				const skip = event.ctrlKey ? CtrlArrowKeySkip : ArrowKeySkip;
				setStartEnd((s) => {
					const next = Math.max(s[0] - skip, 0);
					args.audioRef.currentTime = next;
					return [next, s[1]];
				});
			} else {
				return;
			}
		};
		window.addEventListener("keydown", handleArrowKeys);
		return () => window.removeEventListener("keydown", handleArrowKeys);
	}, [actualDuration, args.audioRef]);

	const startTimer = useCallback(() => {
		clearInterval(args.intervalRef.current);
		args.intervalRef.current = window.setInterval(() => {
			setStartEnd((prev) => [
				Math.min(args.audioRef.currentTime, actualDuration),
				prev[1],
			]);
		}, 1000);
	}, [actualDuration, args.audioRef, args.intervalRef]);

	const togglePlay = useCallback(() => {
		setPlaying((prev) => {
			if (prev) {
				clearInterval(args.intervalRef.current);
				args.audioRef.pause();
				return false;
			}
			args.audioRef.play();
			startTimer();
			return true;
		});
	}, [args.audioRef, args.intervalRef, startTimer]);

	useEffect(() => {
		const handleSpace = (event: KeyboardEvent) => {
			if (event.key !== " " && event.code !== "Space") return;
			const target = event.target as HTMLElement | null;
			if (
				target &&
				(target.tagName === "INPUT" ||
					target.tagName === "TEXTAREA" ||
					target.isContentEditable)
			)
				return;
			event.preventDefault();
			togglePlay();
		};
		window.addEventListener("keydown", handleSpace);
		return () => window.removeEventListener("keydown", handleSpace);
	}, [togglePlay]);

	const handleChange = (
		_event: Event,
		newValue: number | number[],
		activeThumb: number,
	) => {
		if (!Array.isArray(newValue)) {
			if (startEnd[0] + newValue < 0) {
				setStartEnd([0, startEnd[1]]);
				return;
			} else if (startEnd[0] + newValue + MinDistance > startEnd[1]) {
				setStartEnd([startEnd[1] - MinDistance, startEnd[1]]);
				args.audioRef.play();
				return;
			}
			return;
		}

		if (activeThumb === 0) {
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
