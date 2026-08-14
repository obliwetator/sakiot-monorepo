import { describe, expect, test } from "bun:test";
import {
	commonTimelinePosition,
	layoutChannelMixSegment,
} from "./channelMixWaveform";

describe("channel mix waveform layout", () => {
	test("places an aligned fragment and crops its physical waveform", () => {
		expect(
			layoutChannelMixSegment(
				{
					start_ms: 2_000,
					end_ms: 5_000,
					source_offset_ms: 1_000,
					source_duration_ms: 6_000,
				},
				10_000,
			),
		).toEqual({
			leftFraction: 0.2,
			widthFraction: 0.3,
			startFraction: 1 / 6,
			endFraction: 2 / 3,
		});
	});

	test("clips positions and rejects empty segments", () => {
		expect(
			layoutChannelMixSegment(
				{
					start_ms: 4_000,
					end_ms: 4_000,
					source_offset_ms: 0,
					source_duration_ms: 1,
				},
				10_000,
			),
		).toBeNull();
		expect(commonTimelinePosition(-1, 10)).toBe(0);
		expect(commonTimelinePosition(20, 10)).toBe(1);
	});
});
