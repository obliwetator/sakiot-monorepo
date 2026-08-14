import type { ChannelMixSourceSegment } from "../../app/apiSlice";

export interface ChannelMixSegmentLayout {
	leftFraction: number;
	widthFraction: number;
	startFraction: number;
	endFraction: number;
}

export function layoutChannelMixSegment(
	segment: Pick<
		ChannelMixSourceSegment,
		"start_ms" | "end_ms" | "source_offset_ms" | "source_duration_ms"
	>,
	durationMs: number,
): ChannelMixSegmentLayout | null {
	if (!Number.isFinite(durationMs) || durationMs <= 0) return null;
	if (segment.end_ms <= segment.start_ms) return null;
	const leftFraction = Math.max(0, Math.min(1, segment.start_ms / durationMs));
	const rightFraction = Math.max(0, Math.min(1, segment.end_ms / durationMs));
	if (rightFraction <= leftFraction) return null;
	const sourceDuration = Math.max(1, segment.source_duration_ms);
	const startFraction = Math.max(
		0,
		Math.min(1, segment.source_offset_ms / sourceDuration),
	);
	const visibleDuration = segment.end_ms - segment.start_ms;
	const endFraction = Math.max(
		startFraction,
		Math.min(1, (segment.source_offset_ms + visibleDuration) / sourceDuration),
	);
	return {
		leftFraction,
		widthFraction: rightFraction - leftFraction,
		startFraction,
		endFraction,
	};
}

export function commonTimelinePosition(
	positionMs: number,
	durationMs: number,
): number {
	if (!Number.isFinite(durationMs) || durationMs <= 0) return 0;
	return Math.max(0, Math.min(1, positionMs / durationMs));
}
