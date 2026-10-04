import { useEffect, useMemo, useState } from "react";
import { useGetSessionManifestQuery } from "../../app/apiSlice";
import { extendActiveManifest } from "./logicalSessionTimeline";

/** How often an active session's timeline end moves between polls. */
const ACTIVE_TICK_MS = 1_000;

/**
 * Loads a session manifest, polling every 5 s until the session is
 * finalized. Realtime cannot replace this poll: a recording that is growing
 * changes no displayed row (only its heartbeat, which notifies nothing), so
 * no event announces the new audio. Between polls an active session's end
 * keeps moving with the clock (`extendActiveManifest`).
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

	const [now, setNow] = useState(() => Date.now());
	const active = state === "active";
	useEffect(() => {
		if (!active) return;
		const timer = setInterval(() => setNow(Date.now()), ACTIVE_TICK_MS);
		return () => clearInterval(timer);
	}, [active]);
	const fetched = query.data;
	const fetchedAt = query.fulfilledTimeStamp;
	const data = useMemo(
		() =>
			fetched && fetchedAt !== undefined
				? extendActiveManifest(fetched, now - fetchedAt)
				: fetched,
		[fetched, fetchedAt, now],
	);
	return { ...query, data };
}
