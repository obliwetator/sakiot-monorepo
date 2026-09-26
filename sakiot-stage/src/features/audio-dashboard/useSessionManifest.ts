import { useEffect, useState } from "react";
import { useGetSessionManifestQuery } from "../../app/apiSlice";

/** Loads a session manifest, polling while it can still change. */
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
