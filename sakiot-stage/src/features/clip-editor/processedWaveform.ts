import type { WaveformEnvelope } from "../audio-dashboard/waveformPeaks";
import type { SharedDspPcm } from "./sharedDspProtocol";

export const EFFECT_WAVEFORM_POINTS = 2_500;

/** Reduce exact rendered PCM to the same min/max representation used by the
 * server-generated waveform. Both channels contribute to each point.
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
