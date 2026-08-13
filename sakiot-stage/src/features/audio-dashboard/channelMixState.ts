import type { ChannelMixResponse } from "../../app/apiSlice";

const CHANNEL_MIX_STATUSES = [
	"unavailable",
	"waiting",
	"idle",
	"processing",
	"ready",
	"failed",
] as const satisfies readonly ChannelMixResponse["status"][];

export function parseChannelMixStatus(
	value: unknown,
): ChannelMixResponse["status"] | null {
	return typeof value === "string" &&
		CHANNEL_MIX_STATUSES.includes(value as ChannelMixResponse["status"])
		? (value as ChannelMixResponse["status"])
		: null;
}

export function channelMixPollInterval(status: unknown): number {
	const parsed = parseChannelMixStatus(status);
	return parsed === "waiting" || parsed === "processing" ? 1_500 : 0;
}

export function canGenerateChannelMix(
	status: unknown,
	finalized: boolean,
): boolean {
	const parsed = parseChannelMixStatus(status);
	return finalized && (parsed === "idle" || parsed === "failed");
}
