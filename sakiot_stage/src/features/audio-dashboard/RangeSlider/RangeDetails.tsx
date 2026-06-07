import Box from "@mui/material/Box";
import Stack from "@mui/material/Stack";
import type { Dispatch, SetStateAction } from "react";
import type { AudioParams } from "../../../Constants";
import { formatDuration } from "../../../utils/formatTime";
import { PlaybackSpeedSlider } from "./PlaybackSpeedSlider";
import { TimeEditors } from "./TimeEditor";
import { VolumeSlider } from "./VolumeSlider";

export function RangeDetails(props: {
	audioRef: HTMLAudioElement;
	params: Readonly<Partial<Record<AudioParams, string>>>;
	startEnd: number[];
	setStartEnd: Dispatch<SetStateAction<number[]>>;
	durationSec: number;
	onPinEnd: () => void;
	recordingStartedAtMs?: number | null;
}) {
	const absoluteTimeMs =
		props.recordingStartedAtMs == null
			? null
			: props.recordingStartedAtMs + props.startEnd[0] * 1000;

	return (
		<>
			<Stack
				spacing={{ xs: 2, md: 8 }}
				direction={{ xs: "column", md: "row" }}
				alignItems={{ xs: "stretch", md: "center" }}
				justifyContent="space-around"
				sx={{ my: 2 }}
			>
				<VolumeSlider audioRef={props.audioRef} />
				<PlaybackSpeedSlider audioRef={props.audioRef} />
			</Stack>
			<Box
				sx={{
					display: "flex",
					flexDirection: { xs: "column", md: "row" },
					gap: 1,
				}}
			>
				<Box sx={{ flex: 1, minWidth: 0 }}>
					Playback time: {formatDuration(props.startEnd[0])}
					<Box>
						Absolute time:{" "}
						{absoluteTimeMs == null
							? "-"
							: new Date(
									Math.floor(absoluteTimeMs / 1000) * 1000,
								).toLocaleString()}
					</Box>
				</Box>
				<Box sx={{ flex: 1, minWidth: 0 }}>
					Recorded in channel: {props.params.channel_id}
					{(() => {
						const parts = (props.params.file_name ?? "").split("-");
						const userId = parts[1];
						return userId ? (
							<Box sx={{ fontSize: 12, opacity: 0.75 }}>User ID: {userId}</Box>
						) : null;
					})()}
				</Box>
				<Box>
					<TimeEditors
						startEnd={props.startEnd}
						setStartEnd={props.setStartEnd}
						audioRef={props.audioRef}
						durationSec={props.durationSec}
						onPinEnd={props.onPinEnd}
					/>
				</Box>
			</Box>
			<br />
			value 2: {formatDuration(props.startEnd[1])}
			<br />
			Cropped length: {formatDuration(props.startEnd[1] - props.startEnd[0])}
			<br />
			<br />
			<br />
		</>
	);
}
