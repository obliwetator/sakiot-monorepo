import type { ReactNode } from "react";
import { ProgressBar } from "../../shared/ui";

/**
 * The build/unavailable overlay both waveform panels draw over the canvas.
 * The three states are mutually exclusive, error first.
 */
export function WaveformStatusOverlay(props: {
	/** Sentence-cased subject, e.g. "Clip" or the logical recording's name. */
	name: string;
	building: boolean;
	progress: number;
	error: boolean;
	built: boolean;
	/** Appended to the "has not been built" line. */
	notBuiltNote?: ReactNode;
}) {
	if (props.error) {
		return (
			<div className="absolute inset-0 grid place-items-center z-2 pointer-events-none">
				<span className="text-danger text-xs leading-5">
					{props.name} waveform unavailable.
				</span>
			</div>
		);
	}
	if (props.building) {
		return (
			<div className="absolute top-0 left-0 right-0 z-2 px-2 py-1 bg-slate-900/78 pointer-events-none">
				<span className="text-xs leading-5">
					Building {props.name.toLowerCase()} waveform ({props.progress}%)
				</span>
				<ProgressBar value={props.progress} />
			</div>
		);
	}
	if (!props.built) {
		return (
			<div className="absolute inset-0 grid place-items-center z-1 pointer-events-none">
				<span className="text-muted text-xs leading-5">
					{props.name} waveform has not been built.
					{props.notBuiltNote}
				</span>
			</div>
		);
	}
	return null;
}
