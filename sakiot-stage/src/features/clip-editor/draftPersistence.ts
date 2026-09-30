import { browserStorage, type StorageAccess } from "./browserStorage";
import { serializeEdit } from "./composePayload";
import {
	type DraftIdentity,
	type DraftLoadResult,
	type DraftSaveFailure,
	draftFile,
	draftKey,
	loadDraft,
	loadLegacyDraft,
	parseStored,
	readStoredDraftText,
	type StoredDraft,
	saveDraft,
} from "./draftStorage";
import { type ClipEdit, emptyEdit } from "./model";

/**
 * How long a drag or slider preview may stay unwritten. Committed edits are
 * written immediately; previews change on every pointer move, so they are
 * folded into one trailing write in case the gesture never commits.
 */
export const PREVIEW_SAVE_DELAY_MS = 1000;

export type DraftStatus =
	/** Nothing to save: no edit has been made since the editor opened. */
	| { kind: "idle" }
	| { kind: "saving" }
	| { kind: "saved"; savedAt: number }
	| { kind: "failed"; reason: DraftSaveFailure }
	/** Another tab wrote this draft; automatic saving is paused. */
	| { kind: "conflict"; otherVersionReadable: boolean }
	/** The stored draft is unreadable; saving is paused so it survives. */
	| { kind: "damaged" };

export interface DraftSnapshot {
	status: DraftStatus;
	/**
	 * Leaving now loses nothing: the latest edit is stored, or there is no edit
	 * to store. False while a conflict pauses saving.
	 */
	persisted: boolean;
	/** A draft saved before drafts were per user, offered for recovery. */
	legacyDraftOffered: boolean;
}

export interface DraftEnvironment {
	storage(): StorageAccess;
	now(): number;
	/** Identifies this editor session in the records it writes. */
	writer: string;
	/** Schedules `callback`; the returned function cancels it. */
	setTimer(callback: () => void, ms: number): () => void;
}

function newWriterId(): string {
	return (
		globalThis.crypto?.randomUUID?.() ??
		`${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`
	);
}

export function browserDraftEnvironment(): DraftEnvironment {
	return {
		storage: browserStorage,
		now: Date.now,
		writer: newWriterId(),
		setTimer(callback, ms) {
			const handle = setTimeout(callback, ms);
			return () => clearTimeout(handle);
		},
	};
}

type Pause =
	| { kind: "conflict"; theirs: StoredDraft | null }
	| { kind: "damaged"; raw: string };

function sameComposition(a: ClipEdit, b: ClipEdit): boolean {
	return (
		a === b ||
		JSON.stringify(serializeEdit(a)) === JSON.stringify(serializeEdit(b))
	);
}

/**
 * One editor session's draft: restores it on construction, then keeps
 * storage in step with the edit. The editor reports every edit through
 * `update`; `flush` writes synchronously for page hide, navigation and
 * unmount.
 *
 * "No saved draft" and "a saved empty edit" are different states. Without a
 * record the editor opens the source clip (or an empty edit); with one it
 * opens the record, even when the record has no segments. Nothing is written
 * until `start` reports what the editor opened with, so a half-loaded editor
 * never records an empty edit over a clip that was still loading.
 *
 * Every write first checks that the key still holds the text this session
 * last read or wrote. Different text means another tab saved newer work:
 * saving pauses until the user picks a version.
 */
export class DraftPersistence {
	/** The edit the editor must open with: the stored draft, or empty. */
	readonly initialEdit: ClipEdit;
	/** Whether `initialEdit` came from a stored draft. */
	readonly restored: boolean;
	readonly key: string;

	/** The edit whose serialization storage holds, as this session knows. */
	private stored: ClipEdit | null = null;
	/** The exact text under the key, as this session last read or wrote it. */
	private storedRaw: string | null = null;
	private savedAt: number | null = null;
	/** What reopening would show without a record: the seed or the empty edit. */
	private baseline: ClipEdit | null = null;
	private latest: ClipEdit | null = null;
	private committed: ClipEdit | null = null;
	private pause: Pause | null = null;
	private failure: DraftSaveFailure | null = null;
	private cancelTimer: (() => void) | null = null;
	private readonly legacy: ClipEdit | null;
	private legacyOffered: ClipEdit | null = null;
	private snapshot: DraftSnapshot = {
		status: { kind: "idle" },
		persisted: true,
		legacyDraftOffered: false,
	};
	private readonly listeners = new Set<() => void>();

	constructor(
		readonly identity: DraftIdentity,
		private readonly env: DraftEnvironment = browserDraftEnvironment(),
	) {
		this.key = draftKey(identity);
		const loaded = loadDraft(identity, env.storage());
		if (loaded.status === "loaded") {
			this.stored = loaded.draft.edit;
			this.storedRaw = loaded.draft.raw;
			this.savedAt = loaded.draft.savedAt;
		} else if (loaded.status === "malformed") {
			this.storedRaw = loaded.raw;
			this.pause = { kind: "damaged", raw: loaded.raw };
		}
		this.restored = this.stored !== null;
		this.initialEdit = this.stored ?? emptyEdit();
		this.legacy =
			loaded.status === "missing"
				? loadLegacyDraft(
						identity.guildId,
						identity.sourceClipId,
						env.storage(),
					)
				: null;
	}

	readonly subscribe = (listener: () => void): (() => void) => {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	};

	readonly getSnapshot = (): DraftSnapshot => this.snapshot;

	/**
	 * The editor finished opening with `baseline`: the restored draft, the
	 * seeded source clip, or the empty edit. Saving starts here.
	 */
	start(baseline: ClipEdit): void {
		if (this.baseline !== null) return;
		this.baseline = baseline;
		if (this.legacy && !sameComposition(this.legacy, baseline)) {
			this.legacyOffered = this.legacy;
		}
		this.sync();
	}

	/** Reports the editor's latest edit and its last committed history step. */
	update(latest: ClipEdit, committed: ClipEdit): void {
		if (latest === this.latest && committed === this.committed) return;
		this.latest = latest;
		this.committed = committed;
		this.sync();
	}

	/** Writes the latest edit now. Returns whether leaving is safe. */
	flush(): boolean {
		this.write(false);
		return this.snapshot.persisted;
	}

	retry(): void {
		this.failure = null;
		this.write(false);
	}

	/** Re-reads the key after another tab may have written it. */
	externalChange(): void {
		const current = readStoredDraftText(this.identity, this.env.storage());
		if (current === undefined || current === this.storedRaw) return;
		if (current === null) {
			// Site data was cleared: the key holds nobody's work anymore.
			this.stored = null;
			this.storedRaw = null;
			if (this.pause?.kind === "damaged") this.pause = null;
			this.sync();
			return;
		}
		this.enterConflict(current);
	}

	/** Resolves a conflict by overwriting the other tab's version. */
	keepThisVersion(): void {
		if (this.pause?.kind !== "conflict") return;
		this.overwriteStored();
	}

	/**
	 * Resolves a conflict by taking the other tab's version. Returns the edit
	 * the editor must apply, or null when that version is unreadable.
	 */
	takeOtherVersion(): ClipEdit | null {
		if (this.pause?.kind !== "conflict" || !this.pause.theirs) return null;
		const theirs = this.pause.theirs;
		this.pause = null;
		this.failure = null;
		this.stored = theirs.edit;
		this.storedRaw = theirs.raw;
		this.savedAt = theirs.savedAt;
		// The editor applies this edit next; until it does, a flush must not
		// write this tab's older edit over the version just taken.
		this.latest = theirs.edit;
		this.committed = theirs.edit;
		this.publish();
		return theirs.edit;
	}

	/** Replaces an unreadable stored draft with the current edit. */
	replaceDamaged(): void {
		if (this.pause?.kind !== "damaged") return;
		this.overwriteStored();
	}

	/** The unreadable stored draft exactly as stored, for download. */
	damagedFile(): { name: string; text: string } | null {
		if (this.pause?.kind !== "damaged") return null;
		const scope = this.identity.sourceClipId ?? "editor";
		return {
			name: `sakiot-clip-draft-${scope}-unreadable.txt`,
			text: this.pause.raw,
		};
	}

	/** Hands the offered legacy draft to the editor and stops offering it. */
	takeLegacyDraft(): ClipEdit | null {
		const legacy = this.legacyOffered;
		this.legacyOffered = null;
		this.publish();
		return legacy;
	}

	dismissLegacyDraft(): void {
		this.legacyOffered = null;
		this.publish();
	}

	/** A downloadable copy of the latest edit. */
	file(): { name: string; text: string } {
		return draftFile(
			this.identity,
			this.latest ?? this.initialEdit,
			this.env.now(),
		);
	}

	/**
	 * Stops pending work and writes the latest edit. The session stays usable,
	 * because Strict Mode replays effects on the same instance.
	 */
	dispose(): void {
		this.flush();
	}

	private needsSave(): boolean {
		if (this.baseline === null || this.latest === null) return false;
		return this.latest !== (this.stored ?? this.baseline);
	}

	private sync(): void {
		this.cancel();
		if (this.pause || !this.needsSave()) {
			this.publish();
			return;
		}
		if (this.latest === this.committed) {
			this.write(false);
			return;
		}
		this.cancelTimer = this.env.setTimer(() => {
			this.cancelTimer = null;
			this.write(false);
		}, PREVIEW_SAVE_DELAY_MS);
		this.publish();
	}

	private write(force: boolean): void {
		this.cancel();
		const latest = this.latest;
		if (
			this.pause ||
			latest === null ||
			this.baseline === null ||
			(!force && !this.needsSave())
		) {
			this.publish();
			return;
		}
		const savedAt = this.env.now();
		const result = saveDraft(
			this.identity,
			latest,
			{ expectedRaw: this.storedRaw, writer: this.env.writer, savedAt },
			this.env.storage(),
		);
		if (result.status === "saved") {
			this.stored = latest;
			this.storedRaw = result.draft.raw;
			this.savedAt = savedAt;
			this.failure = null;
		} else if (result.status === "conflict") {
			this.enterConflict(result.current);
			return;
		} else {
			this.failure = result.reason;
		}
		this.publish();
	}

	/** Accepts whatever the key holds now as overwritable, then writes. */
	private overwriteStored(): void {
		const current = readStoredDraftText(this.identity, this.env.storage());
		this.pause = null;
		this.failure = null;
		this.stored = null;
		this.storedRaw = current ?? null;
		this.write(true);
	}

	private enterConflict(current: string | DraftLoadResult): void {
		this.cancel();
		const loaded = typeof current === "string" ? parseStored(current) : current;
		if (
			loaded.status === "loaded" &&
			this.latest !== null &&
			this.pause?.kind !== "damaged" &&
			sameComposition(this.latest, loaded.draft.edit)
		) {
			// The other tab stored exactly this tab's edit: nothing to resolve.
			// Adopting its text (not rewriting ours) avoids a write that would
			// in turn look like a change to that tab.
			this.pause = null;
			this.stored = this.latest;
			this.storedRaw = loaded.draft.raw;
			this.savedAt = loaded.draft.savedAt;
			this.publish();
			return;
		}
		this.pause = {
			kind: "conflict",
			theirs: loaded.status === "loaded" ? loaded.draft : null,
		};
		this.publish();
	}

	private cancel(): void {
		this.cancelTimer?.();
		this.cancelTimer = null;
	}

	private status(): DraftStatus {
		if (this.baseline === null) return { kind: "idle" };
		if (this.pause?.kind === "conflict") {
			return {
				kind: "conflict",
				otherVersionReadable: this.pause.theirs !== null,
			};
		}
		if (this.pause?.kind === "damaged") return { kind: "damaged" };
		if (this.needsSave()) {
			return this.failure
				? { kind: "failed", reason: this.failure }
				: { kind: "saving" };
		}
		if (this.stored !== null && this.savedAt !== null) {
			return { kind: "saved", savedAt: this.savedAt };
		}
		return { kind: "idle" };
	}

	private publish(): void {
		const status = this.status();
		const next: DraftSnapshot = {
			status: sameStatus(status, this.snapshot.status)
				? this.snapshot.status
				: status,
			persisted: this.pause?.kind !== "conflict" && !this.needsSave(),
			legacyDraftOffered: this.legacyOffered !== null,
		};
		if (
			next.status === this.snapshot.status &&
			next.persisted === this.snapshot.persisted &&
			next.legacyDraftOffered === this.snapshot.legacyDraftOffered
		) {
			return;
		}
		this.snapshot = next;
		for (const listener of this.listeners) listener();
	}
}

function sameStatus(a: DraftStatus, b: DraftStatus): boolean {
	return JSON.stringify(a) === JSON.stringify(b);
}
