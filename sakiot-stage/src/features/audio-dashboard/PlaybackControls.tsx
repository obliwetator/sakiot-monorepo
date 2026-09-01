import { Pause as PauseIcon, Play as PlayArrowIcon } from "lucide-react";
import { Box, Button, Slider, Stack, Typography } from "../../shared/ui";

export function PlaybackControls(props: {
	playing: boolean;
	onTogglePlay: () => void;
	volume: number;
	onVolumeChange: (volume: number) => void;
	playbackRate: number;
	onPlaybackRateChange: (rate: number) => void;
}) {
	return (
		<Stack
			direction="row"
			spacing={{ xs: 1, md: 2 }}
			alignItems="center"
			sx={{ mt: 1, minWidth: 0, overflowX: "auto" }}
		>
			<Button
				variant="contained"
				onClick={props.onTogglePlay}
				startIcon={props.playing ? <PauseIcon /> : <PlayArrowIcon />}
				className="shrink-0"
			>
				{props.playing ? "Pause" : "Play"}
			</Button>
			<Box sx={{ minWidth: { xs: 96, md: 180 }, flex: "1 1 0%" }}>
				<Typography variant="caption">Volume</Typography>
				<Slider
					aria-label="Playback volume"
					min={0}
					max={1}
					step={0.05}
					value={props.volume}
					onChange={(_event, value) => props.onVolumeChange(Number(value))}
				/>
			</Box>
			<Box sx={{ minWidth: { xs: 96, md: 180 }, flex: "1 1 0%" }}>
				<Typography variant="caption">
					Speed {props.playbackRate.toFixed(2)}×
				</Typography>
				<Slider
					aria-label="Playback speed"
					min={0.5}
					max={2}
					step={0.25}
					value={props.playbackRate}
					onChange={(_event, value) =>
						props.onPlaybackRateChange(Number(value))
					}
				/>
			</Box>
		</Stack>
	);
}
