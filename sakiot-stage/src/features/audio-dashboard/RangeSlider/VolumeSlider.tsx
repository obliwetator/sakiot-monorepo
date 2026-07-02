import VolumeDown from "@mui/icons-material/VolumeDown";
import VolumeMute from "@mui/icons-material/VolumeMute";
import Slider from "@mui/material/Slider";
import Stack from "@mui/material/Stack";
import { useState } from "react";
import { valuetext } from "../../../Constants";

export function VolumeSlider(props: { audioRef: HTMLAudioElement }) {
	const [volume, setVolume] = useState(0.5);
	const [muted, setMuted] = useState(false);

	const handleChangeVolume = (_event: Event, newValue: number | number[]) => {
		setVolume(newValue as number);
		props.audioRef.volume = newValue as number;
	};

	return (
		<Stack
			spacing={2}
			direction="row"
			sx={{ mb: 1, width: { xs: "100%", md: 200 } }}
			alignItems="center"
		>
			{muted ? (
				<VolumeMute
					onClick={() => {
						props.audioRef.muted = false;
						setMuted(false);
					}}
				/>
			) : (
				<VolumeDown
					onClick={() => {
						props.audioRef.muted = true;
						setMuted(true);
					}}
				/>
			)}
			<Slider
				max={1}
				step={0.01}
				getAriaLabel={() => "Minimum distance"}
				value={volume}
				onChange={handleChangeVolume}
				valueLabelDisplay="auto"
				getAriaValueText={valuetext}
			/>
		</Stack>
	);
}
