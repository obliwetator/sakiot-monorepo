import { Volume1 as VolumeDown, VolumeX as VolumeMute } from "lucide-react";
import { useState } from "react";
import { Slider } from "../../../shared/ui";
import type { AudioHandle } from "./audioHandle";

export function VolumeSlider(props: { audio: AudioHandle }) {
	const [volume, setVolume] = useState(0.5);
	const [muted, setMuted] = useState(false);

	const handleChangeVolume = (newValue: number) => {
		setVolume(newValue);
		props.audio.setVolume(newValue);
	};

	return (
		<div className="flex items-center flex-row gap-4 mb-2 w-full min-[900px]:w-50">
			{muted ? (
				<VolumeMute
					onClick={() => {
						props.audio.setMuted(false);
						setMuted(false);
					}}
				/>
			) : (
				<VolumeDown
					onClick={() => {
						props.audio.setMuted(true);
						setMuted(true);
					}}
				/>
			)}
			<Slider
				aria-label="Volume"
				step={0.01}
				value={volume}
				maxValue={1}
				onChange={handleChangeVolume}
			/>
		</div>
	);
}
