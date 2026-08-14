import type {
	ChannelMixParticipantSettings,
	ChannelMixResponse,
} from "../../app/apiSlice";

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
	if (parsed === "processing") return 1_500;
	return 0;
}

export function canGenerateChannelMix(
	status: unknown,
	finalized: boolean,
	serverCanGenerate = finalized,
	settings: readonly ChannelMixParticipantSettings[] = [],
	renderDirty = false,
): boolean {
	const parsed = parseChannelMixStatus(status);
	return (
		finalized &&
		serverCanGenerate &&
		(parsed === "idle" ||
			parsed === "failed" ||
			(parsed === "ready" && renderDirty)) &&
		(settings.length === 0 ||
			settings.some((participant) => !participant.muted))
	);
}

export const CHANNEL_MIX_MIN_GAIN_DB = -60;
export const CHANNEL_MIX_MAX_GAIN_DB = 12;

export function clampChannelMixGain(gainDb: number): number {
	if (!Number.isFinite(gainDb)) return 0;
	return Math.min(
		CHANNEL_MIX_MAX_GAIN_DB,
		Math.max(CHANNEL_MIX_MIN_GAIN_DB, gainDb),
	);
}

export function commonLiveSeekPosition(
	edgesMs: readonly number[],
	behindEdgeMs = 2_000,
): number | null {
	const finiteEdges = edgesMs.filter((edge) => Number.isFinite(edge));
	if (finiteEdges.length === 0) return null;
	return Math.max(0, Math.min(...finiteEdges) - behindEdgeMs);
}
