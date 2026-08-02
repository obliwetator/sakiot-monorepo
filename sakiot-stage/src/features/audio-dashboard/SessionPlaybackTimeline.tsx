import Box from "@mui/material/Box";
import Slider from "@mui/material/Slider";
import Stack from "@mui/material/Stack";
import TextField from "@mui/material/TextField";
import Typography from "@mui/material/Typography";
import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
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
	const durationSeconds = props.durationMs / 1_000;

	return (
		<>
			<TimelineRow label="Waveform" labelAlign="flex-start" sx={{ mb: 1 }}>
				{props.waveform}
			</TimelineRow>

			<TimelineRow label="Position">
				<Box
					sx={{ position: "relative" }}
					onPointerMove={(event) => {
						const bounds = event.currentTarget.getBoundingClientRect();
						const fraction = Math.min(
							1,
							Math.max(
								0,
								(event.clientX - bounds.left) / Math.max(1, bounds.width),
							),
						);
						setHoverMs(fraction * props.durationMs);
					}}
					onPointerLeave={() => setHoverMs(null)}
				>
					<Slider
						aria-label={props.positionAriaLabel}
						min={0}
						max={Math.max(0.001, durationSeconds)}
						step={0.01}
						value={Math.min(durationSeconds, props.positionMs / 1_000)}
						onChange={(_event, value) =>
							props.onSeekPreview(Number(value) * 1_000)
						}
						onChangeCommitted={(_event, value) => {
							props.onSeekPreview(null);
							props.onSeek(Number(value) * 1_000);
						}}
						valueLabelDisplay="off"
						sx={{
							display: "block",
							py: 1.5,
							"& .MuiSlider-thumb, & .MuiSlider-track": {
								transition: "none",
							},
						}}
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
							{formatSessionTimecode(props.positionMs / 1_000, durationSeconds)}{" "}
							/ {formatSessionTimecode(durationSeconds, durationSeconds)}
						</Typography>
						<SessionSeekInput
							positionMs={props.positionMs}
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
