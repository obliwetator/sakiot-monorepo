import { useEffect, useMemo, useState } from "react";
import type { WaveformEnvelope } from "../audio-dashboard/waveformPeaks";
import type { TimelineSegment } from "./model";
import {
	renderSharedSegmentPcm,
	type SharedDspPcm,
	sharedDspRenderKey,
	warmSharedDsp,
} from "./sharedDsp";
import { loadClipBuffer } from "./useClipBuffer";

const EFFECT_WAVEFORM_POINTS = 2_500;
const EFFECT_WAVEFORM_DEBOUNCE_MS = 120;

export interface SegmentWaveformResult {
	peaks: WaveformEnvelope;
	processed: boolean;
}

interface RenderedWaveform {
	key: string;
	sourceId: string;
	reverse: boolean;
	peaks: WaveformEnvelope;
}

/** Mirror an already-rendered envelope without mutating the cached result. */
export function mirrorWaveformEnvelope(
	peaks: WaveformEnvelope,
): WaveformEnvelope {
	return { min: peaks.min.slice().reverse(), max: peaks.max.slice().reverse() };
}

export function resolveSegmentWaveform(
	rendered: RenderedWaveform | null,
	requested: Pick<RenderedWaveform, "key" | "sourceId" | "reverse">,
	fallback: WaveformEnvelope,
): SegmentWaveformResult {
	if (rendered?.sourceId !== requested.sourceId) {
		return { peaks: fallback, processed: false };
	}
	if (rendered.key === requested.key) {
		return { peaks: rendered.peaks, processed: true };
	}
	return {
		peaks:
			rendered.reverse === requested.reverse
				? rendered.peaks
				: mirrorWaveformEnvelope(rendered.peaks),
		processed: true,
	};
}

/** Reduce the exact rendered stereo PCM to the same min/max representation
 * used by server-generated waveforms. Both channels contribute to each point.
 */
export function waveformEnvelopeFromPcm(
	pcm: SharedDspPcm,
	targetPoints = EFFECT_WAVEFORM_POINTS,
): WaveformEnvelope {
	if (
		pcm.channels < 1 ||
		pcm.frames < 1 ||
		pcm.interleaved.length < pcm.channels * pcm.frames ||
		!Number.isInteger(targetPoints) ||
		targetPoints < 1
	) {
		return { min: [], max: [] };
	}
	const pointCount = Math.min(targetPoints, pcm.frames);
	const min = new Array<number>(pointCount);
	const max = new Array<number>(pointCount);
	for (let point = 0; point < pointCount; point += 1) {
		const startFrame = Math.floor((point * pcm.frames) / pointCount);
		const endFrame = Math.max(
			startFrame + 1,
			Math.floor(((point + 1) * pcm.frames) / pointCount),
		);
		let low = 1;
		let high = -1;
		for (let frame = startFrame; frame < endFrame; frame += 1) {
			for (let channel = 0; channel < pcm.channels; channel += 1) {
				const sample = Math.max(
					-1,
					Math.min(1, pcm.interleaved[frame * pcm.channels + channel] ?? 0),
				);
				low = Math.min(low, sample);
				high = Math.max(high, sample);
			}
		}
		min[point] = low;
		max[point] = high;
	}
	return { min, max };
}

/**
 * Rebuild a segment waveform from the same cached WASM output used by
 * playback. After the first successful render, its envelope remains visible
 * until the replacement is ready instead of flashing back to the unprocessed
 * source. A reverse edit mirrors that retained envelope optimistically. The
 * source/server envelope is used only for initial loading, source changes, or
 * when the shared renderer is unavailable.
 */
export function useProcessedSegmentWaveform(
	guildId: string,
	segment: TimelineSegment,
	fallback: WaveformEnvelope,
): SegmentWaveformResult {
	const [rendered, setRendered] = useState<RenderedWaveform | null>(null);
	const renderKey = sharedDspRenderKey(segment);

	useEffect(() => {
		if (!guildId || !segment.sourceId || segment.source !== "clip") return;
		let cancelled = false;
		const timeout = window.setTimeout(() => {
			void Promise.all([
				warmSharedDsp(),
				loadClipBuffer(guildId, segment.sourceId),
			])
				.then(([, source]) => {
					if (cancelled) return;
					const pcm = renderSharedSegmentPcm(source, segment);
					if (!pcm || cancelled) return;
					setRendered({
						key: renderKey,
						sourceId: segment.sourceId,
						reverse: segment.effects.reverse,
						peaks: waveformEnvelopeFromPcm(pcm),
					});
				})
				.catch(() => {
					// Playback already owns the visible WASM failure path. A waveform
					// failure simply retains the server-provided source envelope.
				});
		}, EFFECT_WAVEFORM_DEBOUNCE_MS);
		return () => {
			cancelled = true;
			window.clearTimeout(timeout);
		};
	}, [guildId, renderKey, segment]);

	return useMemo(
		() =>
			resolveSegmentWaveform(
				rendered,
				{
					key: renderKey,
					sourceId: segment.sourceId,
					reverse: segment.effects.reverse,
				},
				fallback,
			),
		[fallback, renderKey, rendered, segment.effects.reverse, segment.sourceId],
	);
}
