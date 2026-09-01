import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import {
	Box,
	Slider,
	Stack,
	LegacyTextField as TextField,
	Typography,
} from "../../shared/ui";
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
			size="small"
			label="Go to"
			value={value}
			error={invalid}
			title={
				invalid
					? `Enter a time from ${formatSessionTimecode(0, durationSeconds)} to ${formatSessionTimecode(durationSeconds, durationSeconds)}`
					: "Enter an exact time and press Enter"
			}
			onFocus={() => {
				setEditing(true);
				dirtyRef.current = false;
			}}
			onChange={(event) => {
				dirtyRef.current = true;
				setValue(event.target.value);
				setInvalid(false);
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
			slotProps={{
				htmlInput: {
					"aria-label": "Seek to exact recording time",
					spellCheck: false,
					style: {
						fontVariantNumeric: "tabular-nums",
						textAlign: "center",
					},
				},
			}}
			sx={{ width: durationSeconds >= 3_600 ? 126 : 106 }}
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
			<TimelineRow label="Waveform" labelAlign="flex-start" sx={{ mb: 1 }}>
				{props.waveform}
			</TimelineRow>

			<TimelineRow label="Position">
				<Box
					sx={{ position: "relative" }}
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
				>
					<Slider
						aria-label={props.positionAriaLabel}
						min={0}
						max={Math.max(0.001, durationSeconds)}
						step={0.01}
						value={Math.min(durationSeconds, displayedPositionMs / 1_000)}
						onChange={(_event, value) =>
							scheduleSeekPreview(Number(value) * 1_000)
						}
						onChangeCommitted={(_event, value) => commitSeek(value)}
						onKeyUp={(event: ReactKeyboardEvent<HTMLInputElement>) => {
							if (
								[
									"ArrowLeft",
									"ArrowRight",
									"ArrowUp",
									"ArrowDown",
									"Home",
									"End",
									"PageUp",
									"PageDown",
								].includes(event.key)
							) {
								commitSeek(Number(event.currentTarget.value));
							}
						}}
						valueLabelDisplay="off"
						style={{ accentColor: "#90caf9" }}
						sx={{ display: "block", py: 1.5, transition: "none" }}
					/>
					{hoverMs !== null && (
						<Typography
							aria-hidden="true"
							variant="caption"
							sx={{
								position: "absolute",
								top: -13,
								left: `${(hoverMs / Math.max(1, props.durationMs)) * 100}%`,
								px: 0.75,
								py: 0.25,
								borderRadius: 0.75,
								bgcolor: "rgba(2, 6, 23, 0.92)",
								color: "info.light",
								fontVariantNumeric: "tabular-nums",
								whiteSpace: "nowrap",
								pointerEvents: "none",
								zIndex: 4,
								transform:
									hoverMs < props.durationMs * 0.1
										? "translateX(4px)"
										: hoverMs > props.durationMs * 0.9
											? "translateX(calc(-100% - 4px))"
											: "translateX(-50%)",
							}}
						>
							{formatSessionTimecode(hoverMs / 1_000, durationSeconds)}
						</Typography>
					)}
				</Box>
			</TimelineRow>

			<TimelineRow sx={{ mb: 1.5 }}>
				<Stack
					direction={{ xs: "column", sm: "row" }}
					justifyContent="space-between"
					spacing={1}
					alignItems={{ xs: "stretch", sm: "center" }}
				>
					<Stack
						direction="row"
						spacing={1}
						alignItems="center"
						flexWrap="wrap"
						useFlexGap
					>
						<Typography
							variant="body2"
							sx={{ fontVariantNumeric: "tabular-nums" }}
						>
							Recording{" "}
							{formatSessionTimecode(
								displayedPositionMs / 1_000,
								durationSeconds,
							)}{" "}
							/ {formatSessionTimecode(durationSeconds, durationSeconds)}
						</Typography>
						<SessionSeekInput
							positionMs={displayedPositionMs}
							durationMs={props.durationMs}
							onSeek={props.onSeek}
						/>
					</Stack>
					{props.rightDetail}
				</Stack>
			</TimelineRow>

			{props.children}
		</>
	);
}
