import type React from "react";
import type { VoiceEvent } from "../../../app/apiSlice";
import { Box, Slider, styled, Typography } from "../../../shared/ui";
import { formatDuration } from "../../../utils/formatTime";
import { AudioEventTimeline } from "../AudioEventTimeline";

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
	recordingStartedAtMs?: number | null;
}) {
	return (
		<>
			<Slider
				className="range-slider-tall"
				max={props.durationSec}
				getAriaLabel={() => "Playback range"}
				value={props.startEnd}
				onChange={props.handleChange}
				valueLabelDisplay="auto"
				valueLabelFormat={(value) => <div>{formatDuration(value)}</div>}
				getAriaValueText={(value) => formatDuration(value)}
				disableSwap
			/>
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
			{props.voiceEvents && props.voiceEvents.length > 0 && (
				<AudioEventTimeline
					events={props.voiceEvents}
					durationMs={props.durationSec * 1_000}
					positionMs={props.startEnd[0] * 1_000}
					startedAtMs={props.recordingStartedAtMs ?? undefined}
					onSeek={(offsetMs) => {
						const offsetSec = offsetMs / 1_000;
						props.audioRef.currentTime = offsetSec;
						props.setStartEnd((current) => [
							offsetSec,
							Math.max(offsetSec, current[1]),
						]);
					}}
				/>
			)}
		</>
	);
}
