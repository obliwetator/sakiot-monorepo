import type { components } from "../../api/openapi";
import {
	browserStorage,
	type StorageAccess,
	type StorageLike,
} from "./browserStorage";
import { deserializeEdit, serializeEdit } from "./composePayload";
import type { ClipEdit } from "./model";

export type { StorageAccess, StorageLike };

type ComposeClipBody = components["schemas"]["ComposeClipBody"];

const PREFIX = "sakiot:clip-editor";
export const DRAFT_RECORD_VERSION = 1;
const DRAFT_FILE_KIND = "sakiot-clip-editor-draft";

/**
 * Whose draft this is. The editor keeps one draft per user, guild and source
 * clip (`?source=...`), or one generic draft per user and guild.
 */
export interface DraftIdentity {
	userId: string;
	guildId: string;
	sourceClipId: string | null;
}

/**
 * The stored form. The composition is the same payload the export sends, so
 * a restore uses the export's own deserializer. `writer` names the editor
 * session (one mount in one tab) that wrote `revision`.
 */
interface DraftRecord {
	version: typeof DRAFT_RECORD_VERSION;
	composition: ComposeClipBody;
	revision: number;
	writer: string;
	savedAt: number;
}

export interface StoredDraft {
	edit: ClipEdit;
	revision: number;
	writer: string;
	savedAt: number;
	/** The exact stored text. Any other text under the key is another writer's. */
	raw: string;
}

export type DraftLoadResult =
	| { status: "missing" }
	| { status: "loaded"; draft: StoredDraft }
	| { status: "malformed"; raw: string }
	| { status: "unavailable" };

export type DraftSaveFailure = "unavailable" | "quota" | "failed";

export type DraftSaveResult =
	| { status: "saved"; draft: StoredDraft }
	/** The key no longer holds `expectedRaw`; nothing was written. */
	| { status: "conflict"; current: DraftLoadResult }
	| { status: "error"; reason: DraftSaveFailure };

export function draftKey(identity: DraftIdentity): string {
	const scope = identity.sourceClipId
		? `clip:${identity.sourceClipId}`
		: "generic";
	return `${PREFIX}:drafts:v${DRAFT_RECORD_VERSION}:${identity.userId}:${identity.guildId}:${scope}`;
}

/**
 * Where drafts lived before they were versioned and scoped per user. They are
 * read for recovery and never written or removed.
 */
export function legacyDraftKey(
	guildId: string,
	sourceClipId: string | null,
): string {
	return sourceClipId
		? `${PREFIX}:${guildId}:clip:${sourceClipId}`
		: `${PREFIX}:${guildId}:draft`;
}

/** Reads this identity's draft, telling "none saved" apart from every failure. */
export function loadDraft(
	identity: DraftIdentity,
	access: StorageAccess = browserStorage(),
): DraftLoadResult {
	if (!access.ok) return { status: "unavailable" };
	let raw: string | null;
	try {
		raw = access.storage.getItem(draftKey(identity));
	} catch {
		return { status: "unavailable" };
	}
	return parseStored(raw);
}

/**
 * Writes `edit` as the next revision, unless the key changed since this
 * session last read or wrote `expectedRaw` - then another tab owns newer
 * work and nothing is written. A key that has vanished (cleared site data)
 * holds nobody's work and may be written.
 */
export function saveDraft(
	identity: DraftIdentity,
	edit: ClipEdit,
	options: { expectedRaw: string | null; writer: string; savedAt: number },
	access: StorageAccess = browserStorage(),
): DraftSaveResult {
	if (!access.ok) return { status: "error", reason: "unavailable" };
	const key = draftKey(identity);
	let current: string | null;
	try {
		current = access.storage.getItem(key);
	} catch (error) {
		return { status: "error", reason: saveFailure(error) };
	}
	if (current !== null && current !== options.expectedRaw) {
		return { status: "conflict", current: parseStored(current) };
	}
	const previous = current === null ? null : parseStored(current);
	const revision =
		(previous?.status === "loaded" ? previous.draft.revision : 0) + 1;
	const record: DraftRecord = {
		version: DRAFT_RECORD_VERSION,
		composition: serializeEdit(edit),
		revision,
		writer: options.writer,
		savedAt: options.savedAt,
	};
	const raw = JSON.stringify(record);
	try {
		access.storage.setItem(key, raw);
	} catch (error) {
		return { status: "error", reason: saveFailure(error) };
	}
	return {
		status: "saved",
		draft: {
			edit,
			revision,
			writer: options.writer,
			savedAt: options.savedAt,
			raw,
		},
	};
}

/** The text currently under the key; undefined when storage cannot be read. */
export function readStoredDraftText(
	identity: DraftIdentity,
	access: StorageAccess = browserStorage(),
): string | null | undefined {
	if (!access.ok) return undefined;
	try {
		return access.storage.getItem(draftKey(identity));
	} catch {
		return undefined;
	}
}

export function parseStored(raw: string | null): DraftLoadResult {
	if (raw === null) return { status: "missing" };
	const draft = parseRecord(raw);
	return draft ? { status: "loaded", draft } : { status: "malformed", raw };
}

function parseRecord(raw: string): StoredDraft | null {
	let data: unknown;
	try {
		data = JSON.parse(raw);
	} catch {
		return null;
	}
	if (typeof data !== "object" || data === null) return null;
	const record = data as Record<string, unknown>;
	const { version, revision, writer, savedAt } = record;
	if (version !== DRAFT_RECORD_VERSION) return null;
	if (
		typeof revision !== "number" ||
		!Number.isSafeInteger(revision) ||
		revision < 1
	) {
		return null;
	}
	if (typeof writer !== "string" || writer.length === 0) return null;
	if (typeof savedAt !== "number" || !Number.isFinite(savedAt)) return null;
	const edit = deserializeEdit(record.composition);
	if (!edit) return null;
	return { edit, revision, writer, savedAt, raw };
}

/**
 * A draft from before drafts were versioned: the bare composition payload.
 * Unreadable legacy data is ignored; it is left in place either way.
 */
export function loadLegacyDraft(
	guildId: string,
	sourceClipId: string | null,
	access: StorageAccess = browserStorage(),
): ClipEdit | null {
	if (!access.ok) return null;
	try {
		const raw = access.storage.getItem(legacyDraftKey(guildId, sourceClipId));
		return raw ? deserializeEdit(JSON.parse(raw) as unknown) : null;
	} catch {
		return null;
	}
}

function saveFailure(error: unknown): DraftSaveFailure {
	const name =
		typeof error === "object" && error !== null && "name" in error
			? error.name
			: null;
	if (name === "QuotaExceededError" || name === "NS_ERROR_DOM_QUOTA_REACHED") {
		return "quota";
	}
	if (name === "SecurityError") return "unavailable";
	return "failed";
}

/** A downloadable copy of an edit that `parseDraftFile` can open again. */
export function draftFile(
	identity: DraftIdentity,
	edit: ClipEdit,
	savedAt: number,
): { name: string; text: string } {
	const stamp = new Date(savedAt).toISOString().replace(/[:.]/g, "-");
	return {
		name: `sakiot-clip-draft-${identity.sourceClipId ?? "editor"}-${stamp}.json`,
		text: JSON.stringify(
			{
				kind: DRAFT_FILE_KIND,
				version: DRAFT_RECORD_VERSION,
				guild_id: identity.guildId,
				source_clip_id: identity.sourceClipId,
				saved_at: savedAt,
				composition: serializeEdit(edit),
			},
			null,
			2,
		),
	};
}

export type DraftFileResult =
	| { ok: true; edit: ClipEdit }
	| { ok: false; message: string };

/**
 * Opens a downloaded draft file. A draft only plays in the server its source
 * clips belong to, so files from another server are refused.
 */
export function parseDraftFile(text: string, guildId: string): DraftFileResult {
	const unreadable = {
		ok: false,
		message: "This file isn't a clip editor draft.",
	} as const;
	let data: unknown;
	try {
		data = JSON.parse(text);
	} catch {
		return unreadable;
	}
	if (typeof data !== "object" || data === null) return unreadable;
	const file = data as Record<string, unknown>;
	if (file.kind !== DRAFT_FILE_KIND) return unreadable;
	if (file.version !== DRAFT_RECORD_VERSION) {
		return {
			ok: false,
			message: "This draft was saved by a newer version of the editor.",
		};
	}
	if (file.guild_id !== guildId) {
		return {
			ok: false,
			message:
				"This draft belongs to another server. Open it from that server's clip editor.",
		};
	}
	const edit = deserializeEdit(file.composition);
	if (!edit) {
		return { ok: false, message: "This draft file is damaged." };
	}
	return { ok: true, edit };
}
