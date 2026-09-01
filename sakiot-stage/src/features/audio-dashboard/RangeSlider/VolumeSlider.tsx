import { Volume1 as VolumeDown, VolumeX as VolumeMute } from "lucide-react";
import { useState } from "react";
import { Slider, Stack } from "../../../shared/ui";

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
				getAriaLabel={() => "Volume"}
				value={volume}
				onChange={handleChangeVolume}
				valueLabelDisplay="auto"
				getAriaValueText={(value) => `${Math.round(value * 100)}%`}
			/>
		</Stack>
	);
}
