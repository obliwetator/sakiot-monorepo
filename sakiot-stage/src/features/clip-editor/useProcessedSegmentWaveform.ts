import { useEffect, useMemo, useState } from "react";
import type { WaveformEnvelope } from "../audio-dashboard/waveformPeaks";
import type { TimelineSegment } from "./model";

export { waveformEnvelopeFromPcm } from "./processedWaveform";

import {
	requestSharedSegmentRender,
	sharedDspRenderKey,
	warmSharedDsp,
} from "./sharedDsp";
import { loadClipBuffer } from "./useClipBuffer";

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

/**
 * Rebuild a segment waveform from the same worker-rendered, cached WASM output
 * used by playback. After the first successful render, its envelope remains
 * visible until the replacement is ready instead of flashing back to the
 * unprocessed source. A reverse edit mirrors that retained envelope
 * optimistically. The source/server envelope is used only for initial loading,
 * source changes, or when the shared renderer is unavailable.
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
				.then(async ([, source]) => {
					if (cancelled) return;
					const render = await requestSharedSegmentRender(source, segment);
					if (!render || cancelled) return;
					setRendered({
						key: renderKey,
						sourceId: segment.sourceId,
						reverse: segment.effects.reverse,
						peaks: render.peaks,
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
