import Box from "@mui/material/Box";
import TextField from "@mui/material/TextField";
import type React from "react";
import { useState } from "react";

type Edge = "start" | "end";

function TimeEditor(props: {
	edge: Edge;
	startEnd: number[];
	setStartEnd: React.Dispatch<React.SetStateAction<number[]>>;
	audioRef: HTMLAudioElement;
	durationSec: number;
	onPinEnd?: () => void;
}) {
	const [isError, setIsError] = useState(false);
	const idx = props.edge === "start" ? 0 : 1;
	const current = props.startEnd[idx];
	const hours = Math.floor(current / 3600);
	const minutes = Math.floor((current % 3600) / 60);
	const seconds = Math.floor((current % 3600) % 60);

	function setStartEndWithTime(start: number, end: number) {
		const minDistance = 1;
		let validEnd = end;
		if (validEnd - start < minDistance) validEnd = start + minDistance;
		if (validEnd > props.durationSec) {
			validEnd = props.durationSec;
			if (validEnd - start < minDistance)
				start = Math.max(0, validEnd - minDistance);
		}
		props.setStartEnd([start, validEnd]);
		props.audioRef.currentTime = start;
	}

	function setNewTime(
		nextValue: number,
		prevValue: number,
		multiplier: number,
	) {
		const diff = Math.abs(nextValue - prevValue) * multiplier;
		const sign = nextValue >= prevValue ? 1 : -1;
		if (props.edge === "start") {
			setStartEndWithTime(props.startEnd[0] + sign * diff, props.startEnd[1]);
		} else {
			setStartEndWithTime(props.startEnd[0], props.startEnd[1] + sign * diff);
			props.onPinEnd?.();
		}
	}

	const fields: {
		label: string;
		value: number;
		multiplier: number;
		clampHour: boolean;
	}[] = [
		{ label: "Hours", value: hours, multiplier: 3600, clampHour: true },
		{ label: "Minutes", value: minutes, multiplier: 60, clampHour: false },
		{ label: "Seconds", value: seconds, multiplier: 1, clampHour: false },
	];

	return (
		<Box>
			<div>{props.edge === "start" ? "Left Slider" : "Right Slider"}</div>
			{fields.map((f) => (
				<TextField
					key={f.label}
					error={isError}
					label={f.label}
					variant="standard"
					type="number"
					value={f.value}
					helperText={isError ? "Out of range" : ""}
					inputProps={{ inputMode: "numeric", pattern: "[0-9]*" }}
					onChange={(e) => {
						const raw = e.target.value;
						if (
							f.clampHour &&
							(raw as unknown as number) * 3600 > props.durationSec
						) {
							setIsError(true);
							props.audioRef.currentTime = props.durationSec;
							props.setStartEnd([
								Math.max(0, props.durationSec - 1),
								props.durationSec,
							]);
							return;
						}
						if (typeof raw !== "string" || raw === "") return;
						setIsError(false);
						const nextValue = parseInt(raw, 10);
						if (nextValue > 60) {
							setIsError(true);
							return;
						}
						setNewTime(nextValue, f.value, f.multiplier);
					}}
				/>
			))}
		</Box>
	);
}

export function TimeEditors(props: {
	startEnd: number[];
	setStartEnd: React.Dispatch<React.SetStateAction<number[]>>;
	audioRef: HTMLAudioElement;
	durationSec: number;
	onPinEnd?: () => void;
}) {
	return (
		<>
			<TimeEditor edge="start" {...props} />
			<TimeEditor edge="end" {...props} />
		</>
	);
}
