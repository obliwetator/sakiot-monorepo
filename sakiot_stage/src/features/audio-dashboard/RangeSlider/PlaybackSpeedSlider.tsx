import Slider from "@mui/material/Slider";
import Stack from "@mui/material/Stack";
import { useState } from "react";
import { valuetext } from "../../../Constants";

export function PlaybackSpeedSlider(props: { audioRef: HTMLAudioElement }) {
	const [playbackSpeed, setPlaybackSpeed] = useState(1);

	const handleChangePlaybackSpeed = (
		_event: Event,
		newValue: number | number[],
	) => {
		setPlaybackSpeed(newValue as number);
		props.audioRef.playbackRate = newValue as number;
	};

	return (
		<Stack
			spacing={2}
			direction="row"
			sx={{ mb: 1, width: { xs: "100%", md: 200 } }}
			alignItems="center"
		>
			<Slider
				max={10}
				step={0.1}
				getAriaLabel={() => "Minimum distance"}
				value={playbackSpeed}
				onChange={handleChangePlaybackSpeed}
				valueLabelDisplay="auto"
				getAriaValueText={valuetext}
			/>
		</Stack>
	);
}
