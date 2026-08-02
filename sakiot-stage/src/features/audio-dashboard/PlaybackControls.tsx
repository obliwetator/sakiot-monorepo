import PauseIcon from "@mui/icons-material/Pause";
import PlayArrowIcon from "@mui/icons-material/PlayArrow";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Slider from "@mui/material/Slider";
import Stack from "@mui/material/Stack";
import Typography from "@mui/material/Typography";

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
			direction={{ xs: "column", md: "row" }}
			spacing={2}
			alignItems="center"
			sx={{ mt: 1 }}
		>
			<Button
				variant="contained"
				onClick={props.onTogglePlay}
				startIcon={props.playing ? <PauseIcon /> : <PlayArrowIcon />}
			>
				{props.playing ? "Pause" : "Play"}
			</Button>
			<Box sx={{ minWidth: 180, flex: 1 }}>
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
			<Box sx={{ minWidth: 180, flex: 1 }}>
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
