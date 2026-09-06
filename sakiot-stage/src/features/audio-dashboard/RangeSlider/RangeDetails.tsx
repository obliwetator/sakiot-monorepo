import type { Dispatch, SetStateAction } from "react";
import type { AudioParams } from "../../../Constants";
import { formatDuration } from "../../../utils/formatTime";
import { PlaybackSpeedSlider } from "./PlaybackSpeedSlider";
import { TimeEditors } from "./TimeEditor";
import { VolumeSlider } from "./VolumeSlider";

export function RangeDetails(props: {
	audioRef: HTMLAudioElement;
	params: Readonly<Partial<Record<AudioParams, string>>>;
	startEnd: number[];
	setStartEnd: Dispatch<SetStateAction<number[]>>;
	durationSec: number;
	onPinEnd: () => void;
	recordingStartedAtMs?: number | null;
}) {
	const absoluteTimeMs =
		props.recordingStartedAtMs == null
			? null
			: props.recordingStartedAtMs + props.startEnd[0] * 1000;

	return (
		<>
			<div className="flex items-center justify-around flex-row gap-2 min-[900px]:gap-16 my-4 min-w-0">
				<div className="min-w-0 [flex:1_1_0%] min-[900px]:[flex:0_1_200px]">
					<VolumeSlider audioRef={props.audioRef} />
				</div>
				<div className="min-w-0 [flex:1_1_0%] min-[900px]:[flex:0_1_200px]">
					<PlaybackSpeedSlider audioRef={props.audioRef} />
				</div>
			</div>
			<div className="flex flex-col min-[900px]:flex-row gap-2">
				<div className="flex-1 min-w-0">
					Playback time: {formatDuration(props.startEnd[0])}
					<div>
						Absolute time:{" "}
						{absoluteTimeMs == null
							? "-"
							: new Date(
									Math.floor(absoluteTimeMs / 1000) * 1000,
								).toLocaleString()}
					</div>
				</div>
				<div className="flex-1 min-w-0">
					Recorded in channel: {props.params.channel_id}
					{(() => {
						const parts = (props.params.file_name ?? "").split("-");
						const userId = parts[1];
						return userId ? (
							<div className="[font-size:12px] [opacity:0.75]">
								User ID: {userId}
							</div>
						) : null;
					})()}
				</div>
				<div>
					<TimeEditors
						startEnd={props.startEnd}
						setStartEnd={props.setStartEnd}
						audioRef={props.audioRef}
						durationSec={props.durationSec}
						onPinEnd={props.onPinEnd}
					/>
				</div>
			</div>
			<br />
			value 2: {formatDuration(props.startEnd[1])}
			<br />
			Cropped length: {formatDuration(props.startEnd[1] - props.startEnd[0])}
			<br />
			<br />
			<br />
		</>
	);
}
