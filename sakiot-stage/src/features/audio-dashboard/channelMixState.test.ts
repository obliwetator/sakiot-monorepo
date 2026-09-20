import { describe, expect, test } from "bun:test";
import {
	canGenerateChannelMix,
	channelMixPollInterval,
	clampChannelMixGain,
	commonLiveSeekPosition,
	parseChannelMixStatus,
	shouldSeekSource,
} from "./channelMixState";

describe("channel mix status parsing", () => {
	test("accepts every server status", () => {
		expect(parseChannelMixStatus("unavailable")).toBe("unavailable");
		expect(parseChannelMixStatus("waiting")).toBe("waiting");
		expect(parseChannelMixStatus("idle")).toBe("idle");
		expect(parseChannelMixStatus("processing")).toBe("processing");
		expect(parseChannelMixStatus("ready")).toBe("ready");
		expect(parseChannelMixStatus("failed")).toBe("failed");
	});

	test("rejects malformed status values", () => {
		expect(parseChannelMixStatus("complete")).toBeNull();
		expect(parseChannelMixStatus(null)).toBeNull();
		expect(parseChannelMixStatus(1)).toBeNull();
	});

	test("polls only while a mix is rendering", () => {
		expect(channelMixPollInterval("waiting")).toBe(0);
		expect(channelMixPollInterval("processing")).toBe(1_500);
		expect(channelMixPollInterval("ready")).toBe(0);
		expect(channelMixPollInterval(undefined)).toBe(0);
	});

	test("only finalized idle or failed mixes can be generated", () => {
		expect(canGenerateChannelMix("idle", true)).toBe(true);
		expect(canGenerateChannelMix("failed", true)).toBe(true);
		expect(canGenerateChannelMix("waiting", true)).toBe(false);
		expect(canGenerateChannelMix("idle", false)).toBe(false);
	});

	test("mute rules control generation", () => {
		expect(
			canGenerateChannelMix("idle", true, true, [
				{ user_id: "1", gain_db: 0, muted: true },
				{ user_id: "2", gain_db: 0, muted: true },
			]),
		).toBe(false);
		expect(clampChannelMixGain(-100)).toBe(-60);
		expect(clampChannelMixGain(100)).toBe(12);
		expect(commonLiveSeekPosition([10_000, 12_000])).toBe(8_000);
		expect(commonLiveSeekPosition([])).toBeNull();
	});
});

describe("channel mix source seeking", () => {
	test("a paused source always seeks to the target", () => {
		expect(shouldSeekSource(0, 100, true)).toBe(true);
		expect(shouldSeekSource(100, 100, true)).toBe(true);
	});

	test("a playing source within tolerance is left alone", () => {
		// Seeking a playing element re-fires canplay; an unconditional seek
		// from the canplay handler feeds itself.
		expect(shouldSeekSource(10, 10, false)).toBe(false);
		expect(shouldSeekSource(10, 10.1, false)).toBe(false);
		expect(shouldSeekSource(10, 9.86, false)).toBe(false);
	});

	test("a playing source seeks only after drifting past the tolerance", () => {
		expect(shouldSeekSource(10, 10.2, false)).toBe(true);
		expect(shouldSeekSource(10, 9.8, false)).toBe(true);
		expect(shouldSeekSource(10, 10.5, false, 1_000)).toBe(false);
	});

	test("non-finite positions never trigger a seek while playing", () => {
		expect(shouldSeekSource(Number.NaN, 10, false)).toBe(false);
		expect(shouldSeekSource(10, Number.NaN, false)).toBe(false);
	});
});
