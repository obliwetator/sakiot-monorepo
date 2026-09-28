import { useEffect, useRef, useState } from "react";
import { problemFromError } from "../../app/apiError";
import {
	type ChannelMixScope,
	useGenerateSessionChannelMixMutation,
	useGetSessionChannelMixQuery,
} from "../../app/apiSlice";
import { channelMixRenderSettingsEqual } from "./channelMixDrafts";
import {
	canGenerateChannelMix,
	channelMixPollInterval,
	parseChannelMixStatus,
} from "./channelMixState";
import { useChannelMixDraft } from "./useChannelMixDraft";

/** A 409 carries the mix status document, whose reason explains the conflict. */
function channelMixFailure(failure: unknown): string {
	if (
		typeof failure === "object" &&
		failure !== null &&
		"status" in failure &&
		failure.status === 409 &&
		"data" in failure
	) {
		const status = parseChannelMixStatusBody(failure.data);
		if (status) return `The channel mix was not generated. ${status}`;
	}
	return `Channel mix generation failed. ${problemFromError(failure, "Try again.").message}`;
}

function parseChannelMixStatusBody(data: unknown): string | null {
	if (typeof data !== "object" || data === null || !("reason" in data))
		return null;
	const reason = data.reason;
	return typeof reason === "object" &&
		reason !== null &&
		"message" in reason &&
		typeof reason.message === "string"
		? reason.message
		: null;
}

/**
 * A session's channel mix: its status (polled only while processing), the
 * participant draft, and generation.
 *
 * `manifestState` is `null` until the session manifest has loaded; the mix is
 * not requested before then.
 */
export function useSessionChannelMix(options: {
	sessionId: string;
	scope: ChannelMixScope;
	manifestState: string | null;
	/** Stops every playback before a new mix replaces the current one. */
	stopPlayback: () => void;
}) {
	const { sessionId, scope, manifestState } = options;
	const [pollingInterval, setPollingInterval] = useState(0);
	const {
		currentData: mix,
		isError: statusError,
		refetch,
	} = useGetSessionChannelMixQuery(
		{ recording_session_id: sessionId, scope },
		{ pollingInterval, skip: manifestState === null },
	);
	const status = mix?.status;
	useEffect(() => {
		setPollingInterval(channelMixPollInterval(status));
	}, [status]);

	const previousManifestStateRef = useRef<string | null>(null);
	useEffect(() => {
		if (
			manifestState === "finalized" &&
			previousManifestStateRef.current !== manifestState
		) {
			// A live mix deliberately stops polling while the anchor is waiting.
			// Refresh once when the manifest becomes final so it can move to idle or
			// ready without keeping a live page on a tight status loop.
			void refetch();
		}
		previousManifestStateRef.current = manifestState;
	}, [manifestState, refetch]);

	const [generateMix, generateMixState] =
		useGenerateSessionChannelMixMutation();
	const draft = useChannelMixDraft(sessionId, mix);
	const [actionError, setActionError] = useState<string | null>(null);

	const parsedStatus = parseChannelMixStatus(status);
	const renderedSettings = mix?.generation_settings?.participants;
	const renderDirty = Boolean(
		renderedSettings &&
			!channelMixRenderSettingsEqual(draft.settings, renderedSettings),
	);
	const canGenerate = canGenerateChannelMix(
		parsedStatus,
		manifestState === "finalized",
		mix?.can_generate ?? false,
		draft.settings,
		renderDirty,
	);

	const generate = async () => {
		setActionError(null);
		options.stopPlayback();
		try {
			await generateMix({
				recording_session_id: sessionId,
				scope,
				body: { participants: draft.settings },
			}).unwrap();
			await refetch();
		} catch (failure) {
			setActionError(channelMixFailure(failure));
		}
	};

	return {
		mix,
		statusError,
		refetch,
		draft,
		renderDirty,
		canGenerate,
		processing: parsedStatus === "processing",
		generating: generateMixState.isLoading,
		generate,
		actionError,
	};
}

export type SessionChannelMix = ReturnType<typeof useSessionChannelMix>;
