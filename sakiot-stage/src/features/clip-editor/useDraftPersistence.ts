import { useEffect, useSyncExternalStore } from "react";
import type { DraftPersistence } from "./draftPersistence";
import { parseDraftFile } from "./draftStorage";
import type { ClipEdit } from "./model";

export type UseDraftPersistenceReturn = ReturnType<typeof useDraftPersistence>;

/**
 * Keeps one editor session's draft in step with the edit and the page
 * lifecycle. The session writes committed edits immediately and previews
 * after a pause; this hook also writes when the page is hidden or the editor
 * unmounts, and reacts to other tabs writing the same draft.
 *
 * The editor remounts when the user, guild or source clip changes, so every
 * session is bound to one draft key: its unmount writes its own draft, and
 * none of its callbacks outlive it.
 */
export function useDraftPersistence(options: {
	session: DraftPersistence;
	edit: ClipEdit;
	committed: ClipEdit;
	/** What the editor opened with; null while the source clip is loading. */
	baseline: ClipEdit | null;
	/**
	 * Replaces the whole edit as one undoable step: another tab's version,
	 * an earlier draft, or an opened draft file.
	 */
	replaceEdit: (edit: ClipEdit) => void;
}) {
	const { session, edit, committed, baseline, replaceEdit } = options;
	const snapshot = useSyncExternalStore(session.subscribe, session.getSnapshot);

	useEffect(() => {
		session.update(edit, committed);
	}, [session, edit, committed]);

	useEffect(() => {
		if (baseline) session.start(baseline);
	}, [session, baseline]);

	useEffect(() => {
		const flush = () => {
			session.flush();
		};
		const recheck = () => {
			session.externalChange();
		};
		const onVisibilityChange = () => {
			if (document.visibilityState === "hidden") flush();
			else recheck();
		};
		const onStorage = (event: StorageEvent) => {
			// A null key means another tab cleared all of storage.
			if (event.key === null || event.key === session.key) recheck();
		};
		window.addEventListener("pagehide", flush);
		window.addEventListener("pageshow", recheck);
		window.addEventListener("storage", onStorage);
		document.addEventListener("visibilitychange", onVisibilityChange);
		return () => {
			window.removeEventListener("pagehide", flush);
			window.removeEventListener("pageshow", recheck);
			window.removeEventListener("storage", onStorage);
			document.removeEventListener("visibilitychange", onVisibilityChange);
			session.dispose();
		};
	}, [session]);

	/** Opens a downloaded draft; resolves to an error message, or null. */
	const openFile = async (file: File): Promise<string | null> => {
		let text: string;
		try {
			text = await file.text();
		} catch {
			return "The file couldn't be read.";
		}
		const result = parseDraftFile(text, session.identity.guildId);
		if (!result.ok) return result.message;
		replaceEdit(result.edit);
		return null;
	};

	return {
		status: snapshot.status,
		persisted: snapshot.persisted,
		legacyDraftOffered: snapshot.legacyDraftOffered,
		flush: () => session.flush(),
		retry: () => session.retry(),
		download: () => downloadTextFile(session.file()),
		downloadDamaged: () => {
			const file = session.damagedFile();
			if (file) downloadTextFile(file);
		},
		keepThisVersion: () => session.keepThisVersion(),
		loadOtherVersion: () => {
			const other = session.takeOtherVersion();
			if (other) replaceEdit(other);
		},
		replaceDamaged: () => session.replaceDamaged(),
		loadLegacyDraft: () => {
			const legacy = session.takeLegacyDraft();
			if (legacy) replaceEdit(legacy);
		},
		dismissLegacyDraft: () => session.dismissLegacyDraft(),
		openFile,
	};
}

function downloadTextFile(file: { name: string; text: string }) {
	const type = file.name.endsWith(".json") ? "application/json" : "text/plain";
	const url = URL.createObjectURL(new Blob([file.text], { type }));
	const link = document.createElement("a");
	link.href = url;
	link.download = file.name;
	link.hidden = true;
	document.body.append(link);
	link.click();
	link.remove();
	// Revoking synchronously can cancel the download in some browsers.
	window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}
