import { useCallback, useEffect, useRef, useState } from "react";
import type { components } from "../../api/openapi";
import {
	isDefiniteRejection,
	problemFromError,
	problemFromQueryError,
} from "../../app/apiError";
import {
	apiSlice,
	useComposeClipMutation,
	useGetAuthDetailsQuery,
	useGetComposeClipStatusQuery,
} from "../../app/apiSlice";
import { useAppDispatch } from "../../app/hooks";
import { useJobPolling } from "../../realtime/status";

type Body = components["schemas"]["ComposeClipBody"];
interface PendingExport {
	key: string;
	body: Body;
	jobId: string | null;
}

function loadPending(key: string): PendingExport | null {
	try {
		const value = JSON.parse(localStorage.getItem(key) ?? "null");
		return value &&
			typeof value.key === "string" &&
			value.body?.segments &&
			(value.jobId === null || typeof value.jobId === "string")
			? value
			: null;
	} catch {
		return null;
	}
}

function persist(key: string, pending: PendingExport | null) {
	try {
		if (pending) localStorage.setItem(key, JSON.stringify(pending));
		else localStorage.removeItem(key);
	} catch {
		/* Export still works when browser storage is unavailable. */
	}
}

export function useCompositionExport(guildId: string) {
	const { data: auth } = useGetAuthDetailsQuery();
	const userId = auth?.user?.user_id;
	const storageKey = userId
		? `sakiot:composition-job:${userId}:${guildId}`
		: null;
	const [pending, setPending] = useState<PendingExport | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [done, setDone] = useState(false);
	const [loadedKey, setLoadedKey] = useState<string | null>(null);
	const attempted = useRef<string | null>(null);
	const currentKey = useRef(storageKey);
	currentKey.current = storageKey;
	const dispatch = useAppDispatch();
	const [compose, { isLoading: starting }] = useComposeClipMutation();

	useEffect(() => {
		setPending(storageKey ? loadPending(storageKey) : null);
		setLoadedKey(storageKey);
		setError(null);
		setDone(false);
		attempted.current = null;
	}, [storageKey]);

	const active = loadedKey === storageKey ? pending : null;
	const jobPolling = useJobPolling(1_000);
	const { currentData: status, error: pollError } =
		useGetComposeClipStatusQuery(
			{ guild_id: guildId, clip_id: active?.jobId ?? "" },
			{ skip: !active?.jobId, pollingInterval: jobPolling },
		);

	const submit = useCallback(
		async (request: PendingExport) => {
			if (!storageKey) return;
			attempted.current = request.key;
			setError(null);
			setDone(false);
			try {
				const result = await compose({
					guild_id: guildId,
					body: request.body,
					idempotency_key: request.key,
				}).unwrap();
				if (currentKey.current !== storageKey) return;
				const next = { ...request, jobId: result.id };
				persist(storageKey, next);
				setPending(next);
			} catch (failure) {
				if (currentKey.current !== storageKey) return;
				const problem = problemFromError(failure);
				if (isDefiniteRejection(problem)) {
					// Nothing was queued, and the editor still holds the edit.
					persist(storageKey, null);
					setPending(null);
					setError(`The export was not started. ${problem.message}`);
				} else {
					// It may have been queued before the answer was lost. Keep the
					// exact request and key: resending them can only find that export.
					setError(
						`Could not confirm whether the export started. ${problem.message} Try again to check; this cannot create a duplicate export.`,
					);
				}
			}
		},
		[compose, guildId, storageKey],
	);

	useEffect(() => {
		if (
			active &&
			!active.jobId &&
			attempted.current !== active.key &&
			!starting
		) {
			void submit(active);
		}
	}, [active, starting, submit]);

	useEffect(() => {
		if (!active?.jobId || !storageKey) return;
		if (status?.status === "ready" || status?.status === "failed") {
			persist(storageKey, null);
			setPending(null);
			setDone(status.status === "ready");
			setError(
				status.status === "failed"
					? `The export failed. ${status.error ?? "No further detail is available."}`
					: null,
			);
			if (status.status === "ready")
				dispatch(apiSlice.util.invalidateTags(["Clips"]));
		} else if (
			pollError &&
			"status" in pollError &&
			[403, 404].includes(Number(pollError.status))
		) {
			persist(storageKey, null);
			setPending(null);
			setError(
				"This export is no longer available. Check your clips before starting another export.",
			);
		}
	}, [active?.jobId, status, pollError, storageKey, dispatch]);

	const begin = useCallback(
		(body: Body) => {
			if (!storageKey || starting || active?.jobId) return;
			const request = active ?? { key: crypto.randomUUID(), body, jobId: null };
			persist(storageKey, request);
			setPending(request);
			void submit(request);
		},
		[active, starting, storageKey, submit],
	);

	const pollProblem = pollError ? problemFromQueryError(pollError) : null;

	const resetMessage = useCallback(() => {
		setError(null);
		setDone(false);
	}, []);
	return {
		begin,
		starting,
		rendering: !!active?.jobId,
		done,
		resetMessage,
		error:
			error ??
			(pollProblem
				? pollProblem.status === 401
					? `${pollProblem.message} Your export is still tracked and resumes once you log in.`
					: "Connection interrupted. Your export is still tracked; reconnecting…"
				: null),
		progress: status?.progress ?? 0,
		stage: status?.stage ?? "queued",
		/** Why the previous attempt failed while the job waits to retry. */
		retryReason:
			status?.status === "queued" || status?.status === "running"
				? (status.error ?? null)
				: null,
	};
}
