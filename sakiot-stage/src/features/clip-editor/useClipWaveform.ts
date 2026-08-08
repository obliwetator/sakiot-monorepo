import { useMemo } from "react";
import { useGetClipWaveformQuery } from "../../app/apiSlice";
import {
	decodeWaveformPeaks,
	EMPTY_WAVEFORM_ENVELOPE,
	type WaveformEnvelope,
} from "../audio-dashboard/waveformPeaks";

/**
 * Decoded waveform peaks for a source clip. RTK caches the query by
 * guild/clip, so every segment referencing the same clip shares one request;
 * only the decode is per caller. Returns an empty envelope while the
 * waveform is missing or still building, which renders nothing.
 */
export function useClipWaveform(
	guildId: string,
	clipId: string,
): WaveformEnvelope {
	const { currentData } = useGetClipWaveformQuery(
		{ guild_id: guildId, clip_id: clipId },
		{ skip: !guildId || !clipId },
	);
	const encoded = currentData?.data;
	return useMemo(
		() => (encoded ? decodeWaveformPeaks(encoded) : EMPTY_WAVEFORM_ENVELOPE),
		[encoded],
	);
}
