import { Pause as PauseIcon, Play as PlayArrowIcon } from "lucide-react";
import { Button, cn, Slider } from "../../shared/ui";

const ROW =
	"flex items-center flex-row gap-2 min-[900px]:gap-4 mt-2 min-w-0 overflow-x-auto";
const FIELD = "min-w-24 min-[900px]:min-w-45 [flex:1_1_0%]";

export function PlaybackControls(props: {
	playing: boolean;
	onTogglePlay: () => void;
	volume: number;
	onVolumeChange: (volume: number) => void;
	playbackRate: number;
	onPlaybackRateChange: (rate: number) => void;
	isDisabled?: boolean;
	/** Accessible names differ per surface and are asserted by the e2e suite. */
	volumeLabel?: string;
	speedLabel?: string;
	/** Overrides the row layout; the clip player stacks these on mobile. */
	className?: string;
	fieldClassName?: string;
}) {
	return (
		<div className={cn(ROW, props.className)}>
			<Button
				className="shrink-0"
				variant="primary"
				isDisabled={props.isDisabled}
				onPress={props.onTogglePlay}
			>
				{props.playing ? <PauseIcon /> : <PlayArrowIcon />}
				{props.playing ? "Pause" : "Play"}
			</Button>
			<div className={cn(FIELD, props.fieldClassName)}>
				<span className="text-xs leading-5">Volume</span>
				<Slider
					aria-label={props.volumeLabel ?? "Playback volume"}
					step={0.05}
					value={props.volume}
					minValue={0}
					maxValue={1}
					onChange={(value) => props.onVolumeChange(Number(value))}
				/>
			</div>
			<div className={cn(FIELD, props.fieldClassName)}>
				<span className="text-xs leading-5">
					Speed {props.playbackRate.toFixed(2)}×
				</span>
				<Slider
					aria-label={props.speedLabel ?? "Playback speed"}
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
