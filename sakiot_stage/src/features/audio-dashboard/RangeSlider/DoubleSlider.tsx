import Box from "@mui/material/Box";
import Slider from "@mui/material/Slider";
import { styled } from "@mui/material/styles";
import Typography from "@mui/material/Typography";
import type React from "react";
import type { VoiceEvent } from "../../../app/apiSlice";
import { valuetext } from "../../../Constants";
import { formatDuration } from "../../../utils/formatTime";
import { VoiceEventMarkers } from "./VoiceEventMarkers";

const TinyText = styled(Typography)({
	fontSize: "0.75rem",
	opacity: 0.38,
	fontWeight: 500,
	letterSpacing: 0.2,
});

export function DoubleSlider(props: {
	startEnd: number[];
	setStartEnd: React.Dispatch<React.SetStateAction<number[]>>;
	handleChange: (
		event: Event,
		newValue: number | number[],
		activeThumb: number,
	) => void;
	audioRef: HTMLAudioElement;
	durationSec: number;
	voiceEvents?: VoiceEvent[];
}) {
	return (
		<>
			<Box sx={{ position: "relative" }}>
				<Slider
					sx={{
						"& .MuiSlider-thumb": {
							height: 25,
							width: 5,
							borderRadius: "1px",
						},
					}}
					max={props.durationSec}
					getAriaLabel={() => "Minimum distance"}
					value={props.startEnd}
					onChange={props.handleChange}
					valueLabelDisplay="auto"
					valueLabelFormat={(value) => <div>{formatDuration(value)}</div>}
					getAriaValueText={valuetext}
					disableSwap
				/>
				{props.voiceEvents && props.voiceEvents.length > 0 && (
					<VoiceEventMarkers
						events={props.voiceEvents}
						durationSec={props.durationSec}
						audioRef={props.audioRef}
					/>
				)}
			</Box>
			<Box
				sx={{
					display: "flex",
					alignItems: "center",
					justifyContent: "space-between",
					mt: -2,
				}}
			>
				<TinyText>{formatDuration(props.startEnd[0])} </TinyText>
				<TinyText>{formatDuration(Math.round(props.durationSec))}</TinyText>
			</Box>
		</>
	);
}
