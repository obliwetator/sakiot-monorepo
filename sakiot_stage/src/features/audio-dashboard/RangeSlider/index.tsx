import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import type React from "react";
import { useParams } from "react-router-dom";
import type { VoiceEvent } from "../../../app/apiSlice";
import type { AudioParams, UserGuilds } from "../../../Constants";
import WaveFormButton from "../Waveform";
import { ClipDialog } from "./ClipDialog";
import { DoubleSlider } from "./DoubleSlider";
import { DownloadButton } from "./DownloadButton";
import { JamIt } from "./JamIt";
import { RangeDetails } from "./RangeDetails";
import { SilenceButton } from "./SilenceButton";
import { useRangeSliderState } from "./useRangeSliderState";

export function RangeSlider(props: {
	audioRef: HTMLAudioElement;
	intervalRef: React.MutableRefObject<number | undefined>;
	isClip: boolean;
	userGuilds: UserGuilds[] | null;
	isSilence: boolean;
	isLive?: boolean;
	trueDuration?: number | null;
	/** Wall-clock recording start (epoch ms) for live HLS sources.
	 * When present, the slider's right edge advances continuously from this
	 * anchor instead of stepping with each new HLS segment. */
	liveStartedAt?: number | null;
	recordingStartedAtMs?: number | null;
	voiceEvents?: VoiceEvent[];
}) {
	const params = useParams<AudioParams>();
	const range = useRangeSliderState({
		audioRef: props.audioRef,
		intervalRef: props.intervalRef,
		trueDuration: props.trueDuration,
		liveStartedAt: props.liveStartedAt,
	});

	return (
		<Box sx={{ mx: { xs: 1, md: 8 }, my: { xs: 1, md: 2 } }}>
			<WaveFormButton
				params={params}
				startEnd={range.startEnd}
				isSilence={props.isSilence}
				isClip={props.isClip}
				actionsSlot={
					<Button variant="contained" onClick={range.togglePlay}>
						{range.playing ? "Pause" : "Play"}
					</Button>
				}
			/>
			<DoubleSlider
				audioRef={props.audioRef}
				handleChange={range.handleChange}
				setStartEnd={range.setStartEnd}
				startEnd={range.startEnd}
				durationSec={range.durationSec}
				voiceEvents={props.voiceEvents}
			/>
			<RangeDetails
				audioRef={props.audioRef}
				params={params}
				startEnd={range.startEnd}
				setStartEnd={range.setStartEnd}
				durationSec={range.durationSec}
				onPinEnd={range.pinEnd}
				recordingStartedAtMs={props.recordingStartedAtMs}
			/>
			<DownloadButton
				isClip={props.isClip}
				isSilence={props.isSilence}
				params={params}
			/>
			<ClipDialog
				params={params}
				startEnd={range.startEnd}
				disabled={props.isClip}
			/>
			<SilenceButton
				params={params}
				isSilence={props.isSilence}
				isLive={props.isLive}
			/>
			<JamIt disabled={props.isClip} userGuilds={props.userGuilds} />
		</Box>
	);
}
