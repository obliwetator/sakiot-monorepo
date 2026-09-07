import type React from "react";
import type { VoiceEvent } from "../../../app/apiSlice";
import { Slider } from "../../../shared/ui";
import { formatDuration } from "../../../utils/formatTime";
import { AudioEventTimeline } from "../AudioEventTimeline";

function TinyText({ children }: { children: React.ReactNode }) {
	return (
		<span className="text-xs font-medium tracking-[0.2px] opacity-[0.38]">
			{children}
		</span>
	);
}

export function DoubleSlider(props: {
	startEnd: number[];
	setStartEnd: React.Dispatch<React.SetStateAction<number[]>>;
	handleChange: (values: number[]) => void;
	audioRef: HTMLAudioElement;
	durationSec: number;
	voiceEvents?: VoiceEvent[];
	recordingStartedAtMs?: number | null;
}) {
	return (
		<>
			<Slider
				aria-label="Playback range"
				value={props.startEnd}
				className="range-slider-tall"
				maxValue={props.durationSec}
				thumbLabels={["Start", "End"]}
				onChange={props.handleChange}
			/>
			<div className="flex items-center justify-between -mt-4">
				<TinyText>{formatDuration(props.startEnd[0])} </TinyText>
				<TinyText>{formatDuration(Math.round(props.durationSec))}</TinyText>
			</div>
			{props.voiceEvents && props.voiceEvents.length > 0 && (
				<AudioEventTimeline
					events={props.voiceEvents}
					durationMs={props.durationSec * 1_000}
					positionMs={props.startEnd[0] * 1_000}
					startedAtMs={props.recordingStartedAtMs ?? undefined}
					onSeek={(offsetMs) => {
						const offsetSec = offsetMs / 1_000;
						props.audioRef.currentTime = offsetSec;
						props.setStartEnd((current) => [
							offsetSec,
							Math.max(offsetSec, current[1]),
						]);
					}}
				/>
			)}
		</>
	);
}
