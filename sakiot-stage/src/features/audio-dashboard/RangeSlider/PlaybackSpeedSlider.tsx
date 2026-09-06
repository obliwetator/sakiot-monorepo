import { useState } from "react";
import { Slider } from "../../../shared/ui";

export function PlaybackSpeedSlider(props: { audioRef: HTMLAudioElement }) {
	const [playbackSpeed, setPlaybackSpeed] = useState(1);

	const handleChangePlaybackSpeed = (newValue: number) => {
		setPlaybackSpeed(newValue);
		props.audioRef.playbackRate = newValue;
	};

	return (
		<div className="flex items-center flex-row gap-4 mb-2 w-full min-[900px]:w-50">
			<Slider
				aria-label="Playback speed"
				step={0.1}
				value={playbackSpeed}
				maxValue={10}
				onChange={handleChangePlaybackSpeed}
			/>
		</div>
	);
}
