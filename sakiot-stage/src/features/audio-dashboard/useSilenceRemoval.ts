import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { BASE_API_URL } from "../../app/apiSlice";
import { authedFetch } from "../../app/authedFetch";
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

function saveBlob(blob: Blob, fileName: string) {
	const url = URL.createObjectURL(blob);
	try {
		const anchor = document.createElement("a");
		anchor.href = url;
		anchor.download = fileName;
		document.body.appendChild(anchor);
		anchor.click();
		anchor.remove();
	} catch {
		window.open(url, "_blank");
	} finally {
		window.setTimeout(() => URL.revokeObjectURL(url), 1_000);
	}
}

export function useSilenceRemoval(options: SilenceRemovalOptions) {
	const [status, setStatus] = useState<SilenceRemovalStatus>({
		status: "idle",
		progress: 0,
	});
	const [mediaUrl, setMediaUrl] = useState<string | null>(null);
	const [message, setMessage] = useState<string | null>(null);
	const [error, setError] = useState<string | null>(null);
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
		if (!options.finalized) return;
		void authedFetch(`audio/sessions/${options.sessionId}/remove-silence`)
			.then(async (response) => {
				if (!response.ok) return;
				const result = parseSilenceRemovalStatus(await response.json());
				if (!cancelled) applyStatus(result);
			})
			.catch(() => {
				// Temporary lookup failure leaves action available.
			});
		return () => {
			cancelled = true;
		};
	}, [applyStatus, options.finalized, options.sessionId]);

	useEffect(() => {
		if (status.status !== "processing") return;
		let cancelled = false;
		let timeout: ReturnType<typeof globalThis.setTimeout> | undefined;
		const poll = async () => {
			try {
				const response = await authedFetch(
					`audio/sessions/${options.sessionId}/remove-silence`,
				);
				if (response.ok) {
					const result = parseSilenceRemovalStatus(await response.json());
					if (cancelled) return;
					applyStatus(result);
					if (result.status !== "processing") return;
				}
			} catch {
				// Background job survives transient polling failures.
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
			const response = await authedFetch(
				`audio/sessions/${options.sessionId}/download`,
			);
			if (!response.ok) {
				setError(`Session download failed (${response.status}).`);
				return;
			}
			saveBlob(await response.blob(), `session-${options.sessionId}.ogg`);
		} catch {
			setError("Session download failed.");
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
				const response = await authedFetch(
					`audio/sessions/${options.sessionId}/remove-silence${force ? "?force=true" : ""}`,
					{
						method: "POST",
						headers: { "Content-Type": "application/json" },
						body: JSON.stringify({}),
					},
				);
				if (!response.ok) {
					requestedRef.current = false;
					setStatus({ status: "idle", progress: 0 });
					setError(`Silence removal failed (${response.status}).`);
					return;
				}
				applyStatus(parseSilenceRemovalStatus(await response.json()));
			} catch {
				requestedRef.current = false;
				setStatus({ status: "idle", progress: 0 });
				setError("Silence removal failed.");
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
			const response = await authedFetch(
				`audio/sessions/${options.sessionId}/silence-free?download=true`,
			);
			if (!response.ok) {
				setError(`Silence-free download failed (${response.status}).`);
				return;
			}
			saveBlob(
				await response.blob(),
				`session-${options.sessionId}-silence-free.ogg`,
			);
		} catch {
			setError("Silence-free download failed.");
		} finally {
			setAction(null);
		}
	}, [options.sessionId]);

	return {
		status,
		mediaUrl,
		message,
		error,
		action,
		downloadSession,
		create,
		downloadSilenceFree,
	};
}
