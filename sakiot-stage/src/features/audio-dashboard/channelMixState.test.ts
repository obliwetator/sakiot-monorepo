import { describe, expect, test } from "bun:test";
import {
	canGenerateChannelMix,
	channelMixPollInterval,
	parseChannelMixStatus,
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

	test("polls only while waiting or processing", () => {
		expect(channelMixPollInterval("waiting")).toBe(1_500);
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
});
