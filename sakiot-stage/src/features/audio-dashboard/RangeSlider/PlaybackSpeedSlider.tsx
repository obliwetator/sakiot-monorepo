import { useState } from "react";
import { Slider } from "../../../shared/ui";
import type { AudioHandle } from "./audioHandle";

export function PlaybackSpeedSlider(props: { audio: AudioHandle }) {
	const [playbackSpeed, setPlaybackSpeed] = useState(1);

	const handleChangePlaybackSpeed = (newValue: number) => {
		setPlaybackSpeed(newValue);
		props.audio.setPlaybackRate(newValue);
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
