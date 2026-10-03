import { useEffect, useState } from "react";
import { useGetSessionManifestQuery } from "../../app/apiSlice";
import { useRealtimeLive } from "../../realtime/status";

/**
 * Loads a session manifest. Realtime refreshes it as the session changes;
 * without realtime it polls every 5 s until the session is finalized.
 */
export function useSessionManifest(sessionId: string) {
	const [finalizedSessionId, setFinalizedSessionId] = useState<string | null>(
		null,
	);
	const realtimeLive = useRealtimeLive();
	const query = useGetSessionManifestQuery(sessionId, {
		pollingInterval:
			realtimeLive || finalizedSessionId === sessionId ? 0 : 5_000,
		refetchOnMountOrArgChange: true,
	});
	const state = query.data?.state;
	useEffect(() => {
		if (state === "finalized") setFinalizedSessionId(sessionId);
	}, [state, sessionId]);
	return query;
}
