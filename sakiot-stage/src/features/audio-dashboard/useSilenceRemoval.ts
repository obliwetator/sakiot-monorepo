import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { API_ROUTES, apiUrl } from "../../api/routes";
import {
	ApiRequestError,
	ensureOk,
	MALFORMED_MESSAGE,
	problemFromError,
	readJson,
} from "../../app/apiError";
import { BASE_API_URL } from "../../app/apiSlice";
import { authedFetch } from "../../app/authedFetch";
import { saveBlob } from "../../app/download";
import {
	isMediaJobStatus,
	MediaJobFailedError,
	MediaJobUnreachableError,
	MediaJobWaitTimeoutError,
	waitForMediaJob,
} from "../../app/mediaJobs";
import {
	parseSilenceRemovalStatus,
	type SilenceRemovalStatus,
} from "./silenceRemovalState";

type SessionAction = "download" | "silence" | "silence-download" | null;

interface SilenceRemovalOptions {
	sessionId: string;
	finalized: boolean;
	openWhenReady: boolean;
	onReady: () => void;
	onUnavailable: () => void;
	onActionError: (message: string | null) => void;
}

/** Polls tolerated before a connection notice; processing continues meanwhile. */
const QUIET_POLL_FAILURES = 3;

const isStatusBody = (value: unknown): value is object =>
	typeof value === "object" && value !== null;

/** Explain a failed session download without implying more than is known. */
function downloadFailure(error: unknown): string {
	if (error instanceof MediaJobFailedError)
		return `The session download could not be prepared. ${error.message}`;
	if (error instanceof MediaJobUnreachableError)
		return `Lost contact while the download was being prepared; the server may still finish it. ${error.problem.message}`;
	if (error instanceof MediaJobWaitTimeoutError)
		return "The session download is taking longer than expected. It may still finish on the server; try again later.";
	return `The session download failed. ${problemFromError(error).message}`;
}

export function useSilenceRemoval(options: SilenceRemovalOptions) {
	const [status, setStatus] = useState<SilenceRemovalStatus>({
		status: "idle",
		progress: 0,
	});
	const [mediaUrl, setMediaUrl] = useState<string | null>(null);
	const [message, setMessage] = useState<string | null>(null);
	const [error, setError] = useState<string | null>(null);
	/** A status-check problem; unlike `error`, it never means processing stopped. */
	const [connectionNotice, setConnectionNotice] = useState<string | null>(null);
	const [action, setAction] = useState<SessionAction>(null);
	const requestedRef = useRef(false);
	const onReadyRef = useRef(options.onReady);
	const onUnavailableRef = useRef(options.onUnavailable);
	const mediaPath = useMemo(
		() =>
			new URL(
				`audio/sessions/${options.sessionId}/silence-free`,
				new URL(BASE_API_URL, window.location.origin),
			).toString(),
		[options.sessionId],
	);

	useEffect(() => {
		onReadyRef.current = options.onReady;
		onUnavailableRef.current = options.onUnavailable;
	});

	const applyStatus = useCallback(
		(result: SilenceRemovalStatus) => {
			setStatus(result);
			if (result.status === "ready") {
				setMediaUrl(mediaPath);
				if (requestedRef.current || options.openWhenReady) {
					const requested = requestedRef.current;
					requestedRef.current = false;
					if (requested && !options.openWhenReady) {
						setMessage("Silence-free session ready.");
					}
					onReadyRef.current();
				}
				return;
			}
			setMediaUrl(null);
			onUnavailableRef.current();
			if (result.status === "failed") {
				requestedRef.current = false;
				setError("Silence removal failed. You can try again.");
			}
		},
		[mediaPath, options.openWhenReady],
	);

	useEffect(() => {
		let cancelled = false;
		requestedRef.current = false;
		setMediaUrl(null);
		setStatus({ status: "idle", progress: 0 });
		setError(null);
		setMessage(null);
		setConnectionNotice(null);
		if (!options.finalized) return;
		void authedFetch(
			apiUrl(API_ROUTES.sessionRemoveSilence, {
				recording_session_id: options.sessionId,
			}),
		)
			.then((response) => ensureOk(response))
			.then((response) => readJson(response, isStatusBody))
			.then((body) => {
				if (!cancelled) applyStatus(parseSilenceRemovalStatus(body));
			})
			.catch((failure: unknown) => {
				// The action stays available; say why the state is unknown.
				if (cancelled) return;
				const problem = problemFromError(failure);
				setConnectionNotice(
					`Could not check for an existing silence-free version. ${problem.message}`,
				);
			});
		return () => {
			cancelled = true;
		};
	}, [applyStatus, options.finalized, options.sessionId]);

	useEffect(() => {
		if (status.status !== "processing") return;
		let cancelled = false;
		let failures = 0;
		let timeout: ReturnType<typeof globalThis.setTimeout> | undefined;
		const poll = async () => {
			try {
				const response = await ensureOk(
					await authedFetch(
						apiUrl(API_ROUTES.sessionRemoveSilence, {
							recording_session_id: options.sessionId,
						}),
					),
				);
				const result = parseSilenceRemovalStatus(
					await readJson(response, isStatusBody),
				);
				if (cancelled) return;
				failures = 0;
				setConnectionNotice(null);
				applyStatus(result);
				if (result.status !== "processing") return;
			} catch (failure) {
				// The server job survives polling failures; keep checking and say
				// so once the interruption is more than a blip.
				failures += 1;
				if (!cancelled && failures >= QUIET_POLL_FAILURES) {
					const problem = problemFromError(failure);
					setConnectionNotice(
						`Lost contact while checking progress. Silence removal continues on the server; retrying… ${problem.message}`,
					);
				}
			}
			if (!cancelled) timeout = globalThis.setTimeout(poll, 1_000);
		};
		timeout = globalThis.setTimeout(poll, 750);
		return () => {
			cancelled = true;
			if (timeout !== undefined) globalThis.clearTimeout(timeout);
		};
	}, [applyStatus, options.sessionId, status.status]);

	const downloadSession = useCallback(async () => {
		setAction("download");
		options.onActionError(null);
		setError(null);
		setMessage(null);
		try {
			const accepted = await ensureOk(
				await authedFetch(
					apiUrl(API_ROUTES.sessionDownload, {
						recording_session_id: options.sessionId,
					}),
					{ headers: { "Idempotency-Key": crypto.randomUUID() } },
				),
			);
			const job = await waitForMediaJob(
				await readJson(accepted, isMediaJobStatus),
			);
			if (!job.result_url) {
				throw new ApiRequestError({
					cause: "malformed",
					status: null,
					kind: null,
					message: MALFORMED_MESSAGE,
				});
			}
			const response = await ensureOk(await authedFetch(job.result_url));
			saveBlob(await response.blob(), `session-${options.sessionId}.ogg`);
		} catch (failure) {
			setError(downloadFailure(failure));
		} finally {
			setAction(null);
		}
	}, [options]);

	const create = useCallback(
		async (force = false) => {
			setAction("silence");
			requestedRef.current = true;
			setStatus({ status: "processing", progress: 0 });
			options.onActionError(null);
			setError(null);
			setMessage(null);
			try {
				const response = await ensureOk(
					await authedFetch(
						`${apiUrl(API_ROUTES.sessionRemoveSilence, {
							recording_session_id: options.sessionId,
						})}${force ? "?force=true" : ""}`,
						{
							method: "POST",
							headers: { "Content-Type": "application/json" },
							body: JSON.stringify({}),
						},
					),
				);
				applyStatus(
					parseSilenceRemovalStatus(await readJson(response, isStatusBody)),
				);
			} catch (failure) {
				requestedRef.current = false;
				setStatus({ status: "idle", progress: 0 });
				// Starting again is safe: the server shares one job per session.
				setError(
					`Silence removal was not started. ${
						problemFromError(failure).message
					}`,
				);
			} finally {
				setAction(null);
			}
		},
		[applyStatus, options],
	);

	const downloadSilenceFree = useCallback(async () => {
		setAction("silence-download");
		setError(null);
		try {
			const response = await ensureOk(
				await authedFetch(
					`${apiUrl(API_ROUTES.sessionSilenceFree, {
						recording_session_id: options.sessionId,
					})}?download=true`,
				),
			);
			saveBlob(
				await response.blob(),
				`session-${options.sessionId}-silence-free.ogg`,
			);
		} catch (failure) {
			setError(
				`The silence-free download failed. ${problemFromError(failure).message}`,
			);
		} finally {
			setAction(null);
		}
	}, [options.sessionId]);

	return {
		status,
		mediaUrl,
		message,
		error,
		connectionNotice,
		action,
		downloadSession,
		create,
		downloadSilenceFree,
	};
}
