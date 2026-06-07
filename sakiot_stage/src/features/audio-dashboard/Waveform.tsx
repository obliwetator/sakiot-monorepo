import Button from "@mui/material/Button";
import { useWavesurfer } from "@wavesurfer/react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Params } from "react-router-dom";
import TimelinePlugin from "wavesurfer.js/dist/plugins/timeline.js";
import {
	useGetClipWaveformQuery,
	useGetWaveformQuery,
} from "../../app/apiSlice";
import type { AudioParams } from "../../Constants";

function WaveFormButton(props: {
	params: Readonly<Params<AudioParams>>;
	startEnd?: number[];
	/** Generate from the silence-free version instead of the original. */
	isSilence?: boolean;
	/** Clip view: waveform comes from the clip's own file, not a recording. */
	isClip?: boolean;
	/** Rendered in the same row, left of the Generate button (e.g. Play). */
	actionsSlot?: React.ReactNode;
}) {
	const containerRef = useRef<HTMLDivElement>(null);
	const [shouldGenerate, setShouldGenerate] = useState(false);
	const [timestamp, setTimestamp] = useState<number | undefined>(undefined);
	const [zoomLevel, setZoomLevel] = useState(1);
	const [_minPxPerSec, _setMinPxPerSec] = useState(1); // default minimum px per second

	const { wavesurfer, isReady } = useWavesurfer({
		container: containerRef,
		waveColor: "#ff00ff", // magenta
		progressColor: "#cc00cc", // darker magenta for progress
		barWidth: 2,
		barGap: 1,
		barRadius: 2,
		interact: false,
		plugins: useMemo(
			() => [
				TimelinePlugin.create({
					// Not providing a container renders it inside the main wavesurfer wrapper
					height: 20,
					timeInterval: 60,
					primaryLabelInterval: 600,
					style: {
						fontSize: "12px",
						color: "#6a6a6a",
					},
				}),
			],
			[],
		),
	});

	// Both hooks must be called (RTK rule); gate each with `skip` by mode.
	// On the clip route, `params.file_name` holds the clip_id.
	const isClip = !!props.isClip;
	const recording = useGetWaveformQuery(
		{
			guild_id: props.params.guild_id ?? "",
			channel_id: props.params.channel_id ?? "",
			year: props.params.year ?? "",
			month: Number(props.params.month),
			file_name: props.params.file_name ?? "",
			timestamp,
			silence: props.isSilence,
		},
		{
			skip: !shouldGenerate || isClip,
			// Poll every 1 second while generating
			pollingInterval: shouldGenerate && !isClip ? 1000 : 0,
		},
	);
	const clip = useGetClipWaveformQuery(
		{
			guild_id: props.params.guild_id ?? "",
			clip_id: props.params.file_name ?? "",
			timestamp,
		},
		{
			skip: !shouldGenerate || !isClip,
			pollingInterval: shouldGenerate && isClip ? 1000 : 0,
		},
	);
	const waveformData = isClip ? clip.data : recording.data;
	const queryError = isClip ? clip.error : recording.error;

	useEffect(() => {
		// Stop generating (polling) if we hit 100% or an error
		if (waveformData?.progress === 100 || waveformData?.error || queryError) {
			setShouldGenerate(false);
		}
	}, [waveformData, queryError]);

	const handleClick = () => {
		setTimestamp(Date.now());
		setShouldGenerate(true);
	};

	useEffect(() => {
		if (waveformData?.data && wavesurfer) {
			try {
				const base64Data = waveformData.data;
				const binaryString = atob(base64Data);
				const len = binaryString.length;
				const bytes = new Uint8Array(len);
				for (let i = 0; i < len; i++) {
					bytes[i] = binaryString.charCodeAt(i);
				}
				const buffer = bytes.buffer;

				const view = new DataView(buffer);
				const flags = view.getUint32(4, true);
				const sampleRate = view.getUint32(8, true);
				const samplesPerPixel = view.getUint32(12, true);
				const length = view.getUint32(16, true);

				const is16Bit = flags === 1;
				const peaks = [];
				let offset = 20;

				for (let i = 0; i < length; i++) {
					if (is16Bit) {
						peaks.push(view.getInt16(offset, true));
						offset += 2;
					} else {
						peaks.push(view.getInt8(offset));
						offset += 1;
					}
				}

				const duration = (length * samplesPerPixel) / sampleRate;
				wavesurfer.load("", [peaks], duration);
			} catch (err) {
				console.error("Error processing complete event:", err);
			}
		}
	}, [waveformData?.data, wavesurfer]);

	let progress: number | null = null;
	if (shouldGenerate) {
		if (!waveformData) {
			progress = 0;
		} else if (waveformData.progress < 100) {
			progress = waveformData.progress;
		}
	}

	const error = queryError
		? "Connection to waveform generation failed."
		: waveformData?.error || null;

	// Update waveform position based on startEnd
	useEffect(() => {
		if (wavesurfer && props.startEnd && props.startEnd.length > 0) {
			const duration = wavesurfer.getDuration();
			if (duration > 0) {
				const time = props.startEnd[0];
				wavesurfer.setTime(time);
			}
		}
	}, [props.startEnd, wavesurfer]);

	const handleZoom = (e: React.ChangeEvent<HTMLInputElement>) => {
		const newZoomLevel = Number(e.target.value);
		setZoomLevel(newZoomLevel);
		if (wavesurfer) {
			wavesurfer.zoom(newZoomLevel);
		}
	};

	// Reset zoom level to initial on waveform ready
	useEffect(() => {
		if (isReady) {
			setZoomLevel(0); // 0 or minimum possible value usually auto-fits the screen
		}
	}, [isReady]);

	return (
		<div style={{ width: "100%", maxWidth: "100%", marginBottom: "20px" }}>
			<div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
				{props.actionsSlot}
				<Button
					variant="contained"
					onClick={handleClick}
					disabled={progress !== null}
				>
					{progress !== null
						? `Generating (${progress}%)`
						: "Generate Waveform"}
				</Button>
			</div>
			<p>
				You have to click every time you want to generate a waveform. (for now).
				Be patient for big files
			</p>
			{error && <p style={{ color: "red", padding: "10px" }}>{error}</p>}
			<div style={{ marginTop: "10px", marginBottom: "10px" }}>
				<label>
					Zoom:{" "}
					<input
						type="range"
						min="0"
						max="10"
						value={zoomLevel}
						onChange={handleZoom}
						style={{ width: "200px" }}
					/>
				</label>
			</div>
			<div
				ref={containerRef}
				style={{
					width: "100%",
					height: "148px",
					backgroundColor: "transparent",
					border: "none",
					borderRadius: "4px",
					paddingBottom: "10px",
				}}
			></div>
		</div>
	);
}

export default WaveFormButton;
