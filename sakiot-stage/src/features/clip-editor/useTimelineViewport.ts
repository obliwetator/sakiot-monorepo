import { type RefObject, useEffect, useState } from "react";
import { type ClipEdit, editDuration, segmentDuration } from "./model";

/**
 * The visible window of the timeline: where it starts, how many seconds it
 * spans, and the zoom and pan gestures that move it. The timeline always
 * extends two seconds past the last segment.
 */
export function useTimelineViewport(
	edit: ClipEdit,
	/** The playhead, which keyboard zoom keeps in place. */
	positionRef: RefObject<number>,
) {
	const [viewStartSec, setViewStartSec] = useState(0);
	const [viewWidthSec, setViewWidthSec] = useState(30);
	const contentDurationSec = editDuration(edit) + 2;
	const timelineDurationSec = Math.max(viewWidthSec, contentDurationSec);
	const viewMaxStartSec = Math.max(0, timelineDurationSec - viewWidthSec);

	useEffect(() => {
		setViewStartSec((current) =>
			Math.min(viewMaxStartSec, Math.max(0, current)),
		);
	}, [viewMaxStartSec]);

	const zoomAt = (factor: number, anchorSec: number) => {
		const nextWidth = Math.max(1, Math.min(120, viewWidthSec * factor));
		const anchorFraction =
			(anchorSec - viewStartSec) / Math.max(1, viewWidthSec);
		const nextStart = anchorSec - anchorFraction * nextWidth;
		const nextMaxStart = Math.max(0, contentDurationSec - nextWidth);
		setViewStartSec(Math.min(nextMaxStart, Math.max(0, nextStart)));
		setViewWidthSec(nextWidth);
	};

	const zoom = (factor: number) => zoomAt(factor, positionRef.current);

	const setViewStart = (startSec: number) => {
		if (!Number.isFinite(startSec)) return;
		setViewStartSec(Math.min(viewMaxStartSec, Math.max(0, startSec)));
	};

	const panView = (deltaSec: number) => {
		if (!Number.isFinite(deltaSec) || deltaSec === 0) return;
		setViewStartSec((current) =>
			Math.min(viewMaxStartSec, Math.max(0, current + deltaSec)),
		);
	};

	const fitView = () => {
		const duration = edit.segments.reduce(
			(max, segment) =>
				Math.max(max, segment.timelineStart + segmentDuration(segment)),
			0,
		);
		setViewStartSec(0);
		setViewWidthSec(Math.max(5, duration + 2));
	};

	return {
		viewStartSec,
		viewWidthSec,
		timelineDurationSec,
		viewMaxStartSec,
		zoom,
		zoomAt,
		setViewStart,
		panView,
		fitView,
	};
}
