import { useEffect, useMemo, useState } from "react";
import type {
	ChannelMixParticipantSettings,
	ChannelMixResponse,
} from "../../app/apiSlice";
import {
	channelMixRenderSettingsEqual,
	mergeChannelMixDraft,
	writeChannelMixDraft,
} from "./channelMixDrafts";

export function useChannelMixDraft(
	sessionId: string,
	mix: ChannelMixResponse | undefined,
) {
	const tracks = mix?.tracks ?? [];
	const serverSettings = mix?.generation_settings?.participants;
	const trackKey = useMemo(
		() => tracks.map((track) => track.user_id).join(","),
		[tracks],
	);
	const [settings, setSettings] = useState<ChannelMixParticipantSettings[]>([]);

	useEffect(() => {
		if (trackKey.length === 0) return;
		setSettings((current) => {
			const merged = mergeChannelMixDraft(
				sessionId,
				tracks,
				serverSettings,
				current,
			);
			if (channelMixRenderSettingsEqual(current, merged)) return current;
			writeChannelMixDraft(sessionId, merged);
			return merged;
		});
	}, [serverSettings, sessionId, trackKey, tracks]);

	const update = (next: ChannelMixParticipantSettings[]) => {
		setSettings(next);
		writeChannelMixDraft(sessionId, next);
	};

	return { settings, setSettings: update };
}
