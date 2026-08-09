import { deserializeEdit, serializeEdit } from "./composePayload";
import type { ClipEdit } from "./model";

const PREFIX = "sakiot:clip-editor";

/**
 * Storage key of a session's working draft: the source clip it was seeded
 * from when editing a composed clip (`?source=...`), or one shared draft for
 * the generic editor. Drafts never leak between clips or guilds.
 */
export function draftKey(guildId: string, sourceClipId: string | null): string {
	return sourceClipId
		? `${PREFIX}:${guildId}:clip:${sourceClipId}`
		: `${PREFIX}:${guildId}:draft`;
}

export interface StorageLike {
	getItem(key: string): string | null;
	setItem(key: string, value: string): void;
}

const defaultStorage = (): StorageLike | null =>
	typeof globalThis.localStorage === "undefined"
		? null
		: globalThis.localStorage;

/**
 * Persists the edit as the composition payload so a reload restores it with
 * the same serializer the export uses. Failures (quota, privacy mode) are
 * swallowed - persistence must never break an edit.
 */
export function saveDraft(
	guildId: string,
	sourceClipId: string | null,
	edit: ClipEdit,
	storage: StorageLike | null = defaultStorage(),
): void {
	if (!storage) return;
	try {
		storage.setItem(
			draftKey(guildId, sourceClipId),
			JSON.stringify(serializeEdit(edit)),
		);
	} catch {
		// Storage is best-effort; ignore quota and availability errors.
	}
}

/** The last saved draft for this session context, or null when none exists. */
export function loadDraft(
	guildId: string,
	sourceClipId: string | null,
	storage: StorageLike | null = defaultStorage(),
): ClipEdit | null {
	if (!storage) return null;
	try {
		const raw = storage.getItem(draftKey(guildId, sourceClipId));
		if (!raw) return null;
		return deserializeEdit(JSON.parse(raw) as unknown);
	} catch {
		return null;
	}
}
