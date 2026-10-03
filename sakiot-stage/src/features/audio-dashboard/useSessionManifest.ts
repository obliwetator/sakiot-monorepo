import { useEffect, useState } from "react";
import { useGetSessionManifestQuery } from "../../app/apiSlice";

/**
 * Loads a session manifest, polling every 5 s until the session is
 * finalized. Realtime cannot replace this poll: a recording that is growing
 * changes no displayed row (only its heartbeat, which notifies nothing), so
 * no event announces the new audio.
 */
export function useSessionManifest(sessionId: string) {
	const [finalizedSessionId, setFinalizedSessionId] = useState<string | null>(
		null,
	);
	const query = useGetSessionManifestQuery(sessionId, {
		pollingInterval: finalizedSessionId === sessionId ? 0 : 5_000,
		refetchOnMountOrArgChange: true,
	});
	const state = query.data?.state;
	useEffect(() => {
		if (state === "finalized") setFinalizedSessionId(sessionId);
	}, [state, sessionId]);
	return query;
}
