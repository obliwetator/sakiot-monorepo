import { describe, expect, test } from "bun:test";
import {
	mirrorWaveformEnvelope,
	resolveSegmentWaveform,
	waveformEnvelopeFromPcm,
} from "./useProcessedSegmentWaveform";

describe("waveformEnvelopeFromPcm", () => {
	test("combines stereo rendered samples into min/max points", () => {
		const envelope = waveformEnvelopeFromPcm(
			{
				channels: 2,
				sampleRate: 48_000,
				frames: 4,
				interleaved: new Float32Array([
					-0.25, 0.5, 0.1, 0.25, -0.75, 0.4, 0.2, 0.9,
				]),
			},
			2,
		);
		expect(envelope.min).toEqual([-0.25, -0.75]);
		expect(envelope.max[0]).toBe(0.5);
		expect(envelope.max[1]).toBeCloseTo(0.9);
	});

	test("clamps over-range DSP output to the drawable envelope", () => {
		expect(
			waveformEnvelopeFromPcm(
				{
					channels: 1,
					sampleRate: 48_000,
					frames: 2,
					interleaved: new Float32Array([-2, 3]),
				},
				1,
			),
		).toEqual({ min: [-1], max: [1] });
	});

	test("rejects malformed or empty PCM", () => {
		expect(
			waveformEnvelopeFromPcm({
				channels: 2,
				sampleRate: 48_000,
				frames: 2,
				interleaved: new Float32Array(3),
			}),
		).toEqual({ min: [], max: [] });
	});
});

describe("mirrorWaveformEnvelope", () => {
	test("reverses time without mutating the cached envelope", () => {
		const original = { min: [-0.8, -0.4, -0.2], max: [0.7, 0.3, 0.1] };
		expect(mirrorWaveformEnvelope(original)).toEqual({
			min: [-0.2, -0.4, -0.8],
			max: [0.1, 0.3, 0.7],
		});
		expect(original).toEqual({
			min: [-0.8, -0.4, -0.2],
			max: [0.7, 0.3, 0.1],
		});
	});

	test("retains and mirrors processed peaks while a reverse render is pending", () => {
		const peaks = { min: [-0.8, -0.2], max: [0.7, 0.1] };
		expect(
			resolveSegmentWaveform(
				{ key: "forward", sourceId: "clip", reverse: false, peaks },
				{ key: "reverse-pending", sourceId: "clip", reverse: true },
				{ min: [-1], max: [1] },
			),
		).toEqual({
			peaks: { min: [-0.2, -0.8], max: [0.1, 0.7] },
			processed: true,
		});
	});

	test("uses the source fallback when the segment source changes", () => {
		const fallback = { min: [-1], max: [1] };
		expect(
			resolveSegmentWaveform(
				{
					key: "old",
					sourceId: "old-clip",
					reverse: false,
					peaks: { min: [-0.5], max: [0.5] },
				},
				{ key: "new", sourceId: "new-clip", reverse: false },
				fallback,
			),
		).toEqual({ peaks: fallback, processed: false });
	});
});
