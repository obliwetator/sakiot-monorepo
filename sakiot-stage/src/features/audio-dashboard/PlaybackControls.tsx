import { Pause as PauseIcon, Play as PlayArrowIcon } from "lucide-react";
import { Button, Slider } from "../../shared/ui";

export function PlaybackControls(props: {
	playing: boolean;
	onTogglePlay: () => void;
	volume: number;
	onVolumeChange: (volume: number) => void;
	playbackRate: number;
	onPlaybackRateChange: (rate: number) => void;
}) {
	return (
		<div className="flex items-center flex-row gap-2 min-[900px]:gap-4 mt-2 min-w-0 overflow-x-auto">
			<Button
				className="shrink-0"
				variant="primary"
				onPress={props.onTogglePlay}
			>
				{props.playing ? <PauseIcon /> : <PlayArrowIcon />}
				{props.playing ? "Pause" : "Play"}
			</Button>
			<div className="min-w-24 min-[900px]:min-w-45 [flex:1_1_0%]">
				<span className="text-xs leading-5">Volume</span>
				<Slider
					aria-label="Playback volume"
					step={0.05}
					value={props.volume}
					minValue={0}
					maxValue={1}
					onChange={(value) => props.onVolumeChange(Number(value))}
				/>
			</div>
			<div className="min-w-24 min-[900px]:min-w-45 [flex:1_1_0%]">
				<span className="text-xs leading-5">
					Speed {props.playbackRate.toFixed(2)}×
				</span>
				<Slider
					aria-label="Playback speed"
					step={0.25}
					value={props.playbackRate}
					minValue={0.5}
					maxValue={2}
					onChange={(value) => props.onPlaybackRateChange(Number(value))}
				/>
			</div>
		</div>
	);
}
