import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import { Slider, TextField } from "../../shared/ui";
import {
	formatSessionTimecode,
	parseSessionTimecode,
} from "../../utils/formatTime";
import { TimelineRow } from "./timelineLayout";

function SessionSeekInput(props: {
	positionMs: number;
	durationMs: number;
	onSeek: (positionMs: number) => void;
}) {
	const durationSeconds = props.durationMs / 1_000;
	const currentTimecode = formatSessionTimecode(
		props.positionMs / 1_000,
		durationSeconds,
	);
	const [value, setValue] = useState(currentTimecode);
	const [editing, setEditing] = useState(false);
	const [invalid, setInvalid] = useState(false);
	const cancelBlurRef = useRef(false);
	const dirtyRef = useRef(false);

	useEffect(() => {
		if (!editing && !invalid) setValue(currentTimecode);
	}, [currentTimecode, editing, invalid]);

	const commit = () => {
		const seconds = parseSessionTimecode(value, durationSeconds);
		if (seconds === null) {
			setInvalid(true);
			return;
		}
		setInvalid(false);
		setValue(formatSessionTimecode(seconds, durationSeconds));
		props.onSeek(seconds * 1_000);
	};

	return (
		<TextField
			label="Go to"
			value={value}
			title={
				invalid
					? `Enter a time from ${formatSessionTimecode(0, durationSeconds)} to ${formatSessionTimecode(durationSeconds, durationSeconds)}`
					: "Enter an exact time and press Enter"
			}
			onFocus={() => {
				setEditing(true);
				dirtyRef.current = false;
			}}
			onBlur={() => {
				setEditing(false);
				if (cancelBlurRef.current) {
					cancelBlurRef.current = false;
					return;
				}
				if (dirtyRef.current) commit();
			}}
			onKeyDown={(event) => {
				if (event.key === "Enter") {
					event.preventDefault();
					if (event.target instanceof HTMLInputElement) event.target.blur();
				} else if (event.key === "Escape") {
					event.preventDefault();
					cancelBlurRef.current = true;
					dirtyRef.current = false;
					setValue(currentTimecode);
					setInvalid(false);
					if (event.target instanceof HTMLInputElement) event.target.blur();
				}
			}}
			className={durationSeconds >= 3_600 ? "w-31.5" : "w-26.5"}
			isInvalid={invalid}
			onChange={(value) => {
				dirtyRef.current = true;
				setValue(value);
				setInvalid(false);
			}}
			aria-label={"Seek to exact recording time"}
			spellCheck={false}
			inputStyle={{
				fontVariantNumeric: "tabular-nums",
				textAlign: "center",
			}}
		/>
	);
}

export function SessionPlaybackTimeline(props: {
	waveform: ReactNode;
	positionMs: number;
	durationMs: number;
	onSeek: (positionMs: number) => void;
	onSeekPreview: (positionMs: number | null) => void;
	positionAriaLabel: string;
	rightDetail?: ReactNode;
	children?: ReactNode;
}) {
	const [hoverMs, setHoverMs] = useState<number | null>(null);
	const [dragPositionMs, setDragPositionMs] = useState<number | null>(null);
	const hoverMsRef = useRef<number | null>(null);
	const hoverFrameRef = useRef<number | null>(null);
	const dragPositionRef = useRef<number | null>(null);
	const previewFrameRef = useRef<number | null>(null);
	const draggingRef = useRef(false);
	const durationSeconds = props.durationMs / 1_000;
	const displayedPositionMs = dragPositionMs ?? props.positionMs;

	useEffect(
		() => () => {
			if (hoverFrameRef.current !== null) {
				cancelAnimationFrame(hoverFrameRef.current);
			}
			if (previewFrameRef.current !== null) {
				cancelAnimationFrame(previewFrameRef.current);
			}
		},
		[],
	);

	const clearHover = () => {
		hoverMsRef.current = null;
		if (hoverFrameRef.current !== null) {
			cancelAnimationFrame(hoverFrameRef.current);
			hoverFrameRef.current = null;
		}
		setHoverMs((current) => (current === null ? current : null));
	};

	const scheduleHover = (nextHoverMs: number) => {
		if (draggingRef.current) return;
		hoverMsRef.current = nextHoverMs;
		if (hoverFrameRef.current !== null) return;
		hoverFrameRef.current = requestAnimationFrame(() => {
			hoverFrameRef.current = null;
			if (draggingRef.current) return;
			const next = hoverMsRef.current;
			setHoverMs((current) => (current === next ? current : next));
		});
	};

	const scheduleSeekPreview = (nextPositionMs: number) => {
		const clamped = Math.min(props.durationMs, Math.max(0, nextPositionMs));
		dragPositionRef.current = clamped;
		if (previewFrameRef.current !== null) return;
		previewFrameRef.current = requestAnimationFrame(() => {
			previewFrameRef.current = null;
			const next = dragPositionRef.current;
			if (next === null) return;
			setDragPositionMs((current) => (current === next ? current : next));
			props.onSeekPreview(next);
		});
	};

	const cancelSeek = () => {
		if (previewFrameRef.current !== null) {
			cancelAnimationFrame(previewFrameRef.current);
			previewFrameRef.current = null;
		}
		dragPositionRef.current = null;
		setDragPositionMs(null);
		props.onSeekPreview(null);
	};

	const commitSeek = (value: number | number[]) => {
		if (previewFrameRef.current !== null) {
			cancelAnimationFrame(previewFrameRef.current);
			previewFrameRef.current = null;
		}
		const nextPositionMs = dragPositionRef.current ?? Number(value) * 1_000;
		dragPositionRef.current = null;
		setDragPositionMs(null);
		props.onSeekPreview(null);
		props.onSeek(nextPositionMs);
	};

	return (
		<>
			<TimelineRow label="Waveform" labelAlign="flex-start" className="mb-2">
				{props.waveform}
			</TimelineRow>

			<TimelineRow label="Position">
				<div
					onPointerDown={(event) => {
						const target = event.target;
						if (target instanceof HTMLInputElement && target.type === "range") {
							draggingRef.current = true;
							clearHover();
						}
					}}
					onPointerUp={() => {
						draggingRef.current = false;
					}}
					onPointerCancel={() => {
						draggingRef.current = false;
						cancelSeek();
					}}
					onPointerMove={(event) => {
						if (draggingRef.current) return;
						const bounds = event.currentTarget.getBoundingClientRect();
						const fraction = Math.min(
							1,
							Math.max(
								0,
								(event.clientX - bounds.left) / Math.max(1, bounds.width),
							),
						);
						scheduleHover(fraction * props.durationMs);
					}}
					onPointerLeave={() => {
						if (!draggingRef.current) clearHover();
					}}
					className="relative"
				>
					<Slider
						aria-label={props.positionAriaLabel}
						step={0.01}
						value={Math.min(durationSeconds, displayedPositionMs / 1_000)}
						className="block py-3 transition-none"
						minValue={0}
						maxValue={Math.max(0.001, durationSeconds)}
						onChangeEnd={(value) => commitSeek(value)}
						onChange={(value) => scheduleSeekPreview(Number(value) * 1_000)}
					/>
					{hoverMs !== null && (
						<span
							aria-hidden="true"
							className="text-xs leading-5 absolute -top-[13px] px-1.5 py-0.5 rounded-[0.75px] bg-slate-900/92 text-sky-300 tabular-nums whitespace-nowrap pointer-events-none z-4"
							style={{
								left: `${(hoverMs / Math.max(1, props.durationMs)) * 100}%`,
								transform:
									hoverMs < props.durationMs * 0.1
										? "translateX(4px)"
										: hoverMs > props.durationMs * 0.9
											? "translateX(calc(-100% - 4px))"
											: "translateX(-50%)",
							}}
						>
							{formatSessionTimecode(hoverMs / 1_000, durationSeconds)}
						</span>
					)}
				</div>
			</TimelineRow>

			<TimelineRow className="mb-3">
				<div className="flex justify-between items-stretch min-[600px]:items-center flex-col min-[600px]:flex-row gap-2">
					<div className="flex items-center flex-wrap flex-row gap-2">
						<p className="text-sm tabular-nums">
							Recording{" "}
							{formatSessionTimecode(
								displayedPositionMs / 1_000,
								durationSeconds,
							)}{" "}
							/ {formatSessionTimecode(durationSeconds, durationSeconds)}
						</p>
						<SessionSeekInput
							positionMs={displayedPositionMs}
							durationMs={props.durationMs}
							onSeek={props.onSeek}
						/>
					</div>
					{props.rightDetail}
				</div>
			</TimelineRow>

			{props.children}
		</>
	);
}
