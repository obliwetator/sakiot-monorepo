import { describe, expect, test } from "bun:test";
import type { StorageAccess, StorageLike } from "./browserStorage";
import { serializeEdit } from "./composePayload";
import {
	type DraftEnvironment,
	DraftPersistence,
	PREVIEW_SAVE_DELAY_MS,
} from "./draftPersistence";
import {
	type DraftIdentity,
	draftKey,
	legacyDraftKey,
	loadDraft,
} from "./draftStorage";
import { addSegment, type ClipEdit, emptyEdit, makeSegment } from "./model";

const IDENTITY: DraftIdentity = {
	userId: "user-1",
	guildId: "42",
	sourceClipId: "clip-9",
};

/** Shared browser storage, as every tab of one origin sees it. */
function sharedStorage(initial: Record<string, string> = {}) {
	const values = new Map(Object.entries(initial));
	let writeError: unknown = null;
	const storage: StorageLike = {
		getItem: (key) => values.get(key) ?? null,
		setItem: (key, value) => {
			if (writeError) throw writeError;
			values.set(key, value);
		},
	};
	return {
		values,
		access: (): StorageAccess => ({ ok: true, storage }),
		failWritesWith(error: unknown) {
			writeError = error;
		},
	};
}

/** One tab: its own writer id, clock and timers over the shared storage. */
function tab(storage: () => StorageAccess, writer: string) {
	const timers = new Set<() => void>();
	let now = 1_000;
	const env: DraftEnvironment = {
		storage,
		now: () => now,
		writer,
		setTimer(callback, ms) {
			expect(ms).toBe(PREVIEW_SAVE_DELAY_MS);
			timers.add(callback);
			return () => timers.delete(callback);
		},
	};
	return {
		env,
		pendingTimers: () => timers.size,
		runTimers() {
			now += PREVIEW_SAVE_DELAY_MS;
			for (const callback of [...timers]) {
				timers.delete(callback);
				callback();
			}
		},
	};
}

function withClip(sourceId: string, seconds = 2): ClipEdit {
	return addSegment(
		emptyEdit(),
		makeSegment("clip", sourceId, 0, seconds, 0, 0),
	);
}

function restoredComposition(
	storage: () => StorageAccess,
	identity = IDENTITY,
) {
	const loaded = loadDraft(identity, storage());
	return loaded.status === "loaded"
		? serializeEdit(loaded.draft.edit)
		: loaded.status;
}

/** Commits `edit` the way the editor reports a finished history step. */
function commit(session: DraftPersistence, edit: ClipEdit) {
	session.update(edit, edit);
}

describe("DraftPersistence", () => {
	test("deleting the final segment saves an empty draft that reopening keeps", () => {
		const storage = sharedStorage();
		const seed = withClip("clip-9");
		const first = new DraftPersistence(IDENTITY, tab(storage.access, "a").env);
		expect(first.restored).toBe(false);
		commit(first, first.initialEdit);
		first.start(seed);
		commit(first, seed);
		const emptied = { ...seed, segments: [] };
		commit(first, emptied);
		expect(first.getSnapshot().status.kind).toBe("saved");

		const reopened = new DraftPersistence(
			IDENTITY,
			tab(storage.access, "b").env,
		);
		expect(reopened.restored).toBe(true);
		expect(reopened.initialEdit.segments).toEqual([]);
	});

	test("saves nothing before the editor has opened", () => {
		const storage = sharedStorage();
		const session = new DraftPersistence(
			IDENTITY,
			tab(storage.access, "a").env,
		);
		commit(session, session.initialEdit);
		expect(session.flush()).toBe(true);
		session.dispose();
		expect(storage.values.size).toBe(0);
		expect(session.getSnapshot()).toMatchObject({
			status: { kind: "idle" },
			persisted: true,
		});
	});

	test("opening the source clip is not an edit; the first change is", () => {
		const storage = sharedStorage();
		const seed = withClip("clip-9");
		const session = new DraftPersistence(
			IDENTITY,
			tab(storage.access, "a").env,
		);
		commit(session, seed);
		session.start(seed);
		expect(storage.values.size).toBe(0);
		expect(session.getSnapshot().status.kind).toBe("idle");

		const trimmed = withClip("clip-9", 1);
		commit(session, trimmed);
		expect(restoredComposition(storage.access)).toEqual(serializeEdit(trimmed));
		expect(session.getSnapshot()).toMatchObject({
			status: { kind: "saved", savedAt: 1_000 },
			persisted: true,
		});
	});

	test("writes committed edits at once and previews after a pause", () => {
		const storage = sharedStorage();
		const clock = tab(storage.access, "a");
		const session = new DraftPersistence(IDENTITY, clock.env);
		const base = session.initialEdit;
		session.start(base);
		const committed = withClip("clip-1");
		commit(session, committed);
		expect(restoredComposition(storage.access)).toEqual(
			serializeEdit(committed),
		);

		const dragging = withClip("clip-1", 1.5);
		session.update(dragging, committed);
		expect(clock.pendingTimers()).toBe(1);
		expect(session.getSnapshot()).toMatchObject({
			status: { kind: "saving" },
			persisted: false,
		});
		expect(restoredComposition(storage.access)).toEqual(
			serializeEdit(committed),
		);
		clock.runTimers();
		expect(restoredComposition(storage.access)).toEqual(
			serializeEdit(dragging),
		);
		expect(session.getSnapshot().status.kind).toBe("saved");
	});

	test("flush writes a pending preview synchronously", () => {
		const storage = sharedStorage();
		const clock = tab(storage.access, "a");
		const session = new DraftPersistence(IDENTITY, clock.env);
		session.start(session.initialEdit);
		const dragging = withClip("clip-1");
		session.update(dragging, session.initialEdit);
		expect(session.flush()).toBe(true);
		expect(clock.pendingTimers()).toBe(0);
		expect(restoredComposition(storage.access)).toEqual(
			serializeEdit(dragging),
		);
	});

	test("undo, redo and restore-original each leave their own state stored", () => {
		const storage = sharedStorage();
		const session = new DraftPersistence(
			IDENTITY,
			tab(storage.access, "a").env,
		);
		const seed = withClip("clip-9");
		commit(session, seed);
		session.start(seed);
		const edited = { ...seed, masterVolumeDb: -6 };
		commit(session, edited);
		// Undo returns the earlier history object: the seed.
		commit(session, seed);
		expect(restoredComposition(storage.access)).toEqual(serializeEdit(seed));
		// Redo returns the edited object again.
		commit(session, edited);
		expect(restoredComposition(storage.access)).toEqual(serializeEdit(edited));
		// Restore-original applies a fresh copy of the seed.
		const original = withClip("clip-9");
		commit(session, original);
		expect(restoredComposition(storage.access)).toEqual(
			serializeEdit(original),
		);
	});

	test("never reports a failed write as saved, and retry saves the latest edit", () => {
		const storage = sharedStorage();
		const session = new DraftPersistence(
			IDENTITY,
			tab(storage.access, "a").env,
		);
		session.start(session.initialEdit);
		storage.failWritesWith(new DOMException("full", "QuotaExceededError"));
		commit(session, withClip("clip-1"));
		expect(session.getSnapshot()).toMatchObject({
			status: { kind: "failed", reason: "quota" },
			persisted: false,
		});
		const latest = withClip("clip-2");
		commit(session, latest);
		expect(session.getSnapshot().status.kind).toBe("failed");
		expect(session.flush()).toBe(false);

		storage.failWritesWith(null);
		session.retry();
		expect(session.getSnapshot()).toMatchObject({
			status: { kind: "saved" },
			persisted: true,
		});
		expect(restoredComposition(storage.access)).toEqual(serializeEdit(latest));
	});

	test("unavailable storage restores nothing and fails every save", () => {
		const blocked = (): StorageAccess => ({
			ok: false,
			error: new DOMException("denied", "SecurityError"),
		});
		const session = new DraftPersistence(IDENTITY, tab(blocked, "a").env);
		expect(session.restored).toBe(false);
		session.start(session.initialEdit);
		commit(session, withClip("clip-1"));
		expect(session.getSnapshot()).toMatchObject({
			status: { kind: "failed", reason: "unavailable" },
			persisted: false,
		});
	});

	test("malformed stored data pauses saving until the user replaces it", () => {
		const key = draftKey(IDENTITY);
		const storage = sharedStorage({ [key]: "{not json" });
		const session = new DraftPersistence(
			IDENTITY,
			tab(storage.access, "a").env,
		);
		expect(session.restored).toBe(false);
		expect(session.damagedFile()?.text).toBe("{not json");
		const seed = withClip("clip-9");
		commit(session, seed);
		session.start(seed);
		expect(session.getSnapshot()).toMatchObject({
			status: { kind: "damaged" },
			persisted: true,
		});
		const edited = withClip("clip-9", 1);
		commit(session, edited);
		expect(session.flush()).toBe(false);
		session.dispose();
		expect(storage.values.get(key)).toBe("{not json");

		session.replaceDamaged();
		expect(restoredComposition(storage.access)).toEqual(serializeEdit(edited));
		expect(session.getSnapshot().status.kind).toBe("saved");
	});

	test("another tab's write pauses saving instead of being overwritten", () => {
		const storage = sharedStorage();
		const tabA = new DraftPersistence(IDENTITY, tab(storage.access, "a").env);
		tabA.start(tabA.initialEdit);
		commit(tabA, withClip("from-a"));

		const tabB = new DraftPersistence(IDENTITY, tab(storage.access, "b").env);
		expect(tabB.restored).toBe(true);
		tabB.start(tabB.initialEdit);
		commit(tabB, tabB.initialEdit);

		const newerA = withClip("from-a", 1);
		commit(tabA, newerA);
		tabB.externalChange();
		expect(tabB.getSnapshot()).toMatchObject({
			status: { kind: "conflict", otherVersionReadable: true },
			persisted: false,
		});
		commit(tabB, withClip("from-b"));
		expect(tabB.flush()).toBe(false);
		expect(restoredComposition(storage.access)).toEqual(serializeEdit(newerA));

		const taken = tabB.takeOtherVersion();
		expect(taken && serializeEdit(taken)).toEqual(serializeEdit(newerA));
		if (taken) commit(tabB, taken);
		expect(tabB.getSnapshot()).toMatchObject({
			status: { kind: "saved" },
			persisted: true,
		});
	});

	test("a write that finds unseen text is refused even without a storage event", () => {
		const storage = sharedStorage();
		const tabA = new DraftPersistence(IDENTITY, tab(storage.access, "a").env);
		const tabB = new DraftPersistence(IDENTITY, tab(storage.access, "b").env);
		tabA.start(tabA.initialEdit);
		tabB.start(tabB.initialEdit);
		const fromA = withClip("from-a");
		commit(tabA, fromA);
		commit(tabB, withClip("from-b"));
		expect(tabB.getSnapshot().status.kind).toBe("conflict");
		expect(restoredComposition(storage.access)).toEqual(serializeEdit(fromA));
	});

	test("keeping this tab's version overwrites, and the other tab is told", () => {
		const storage = sharedStorage();
		const tabA = new DraftPersistence(IDENTITY, tab(storage.access, "a").env);
		const tabB = new DraftPersistence(IDENTITY, tab(storage.access, "b").env);
		tabA.start(tabA.initialEdit);
		tabB.start(tabB.initialEdit);
		commit(tabA, withClip("from-a"));
		const fromB = withClip("from-b");
		commit(tabB, fromB);
		tabB.keepThisVersion();
		expect(restoredComposition(storage.access)).toEqual(serializeEdit(fromB));
		expect(tabB.getSnapshot().status.kind).toBe("saved");

		tabA.externalChange();
		expect(tabA.getSnapshot().status.kind).toBe("conflict");
	});

	test("another tab storing this tab's exact edit is not a conflict", () => {
		const storage = sharedStorage();
		const tabA = new DraftPersistence(IDENTITY, tab(storage.access, "a").env);
		tabA.start(tabA.initialEdit);
		const shared = withClip("clip-1");
		commit(tabA, shared);
		const tabB = new DraftPersistence(IDENTITY, tab(storage.access, "b").env);
		commit(tabB, tabB.initialEdit);
		tabB.start(tabB.initialEdit);
		commit(tabA, withClip("clip-1", 1));
		commit(tabA, withClip("clip-1"));
		tabB.externalChange();
		expect(tabB.getSnapshot().status.kind).toBe("saved");
	});

	test("cleared site data is written again", () => {
		const storage = sharedStorage();
		const session = new DraftPersistence(
			IDENTITY,
			tab(storage.access, "a").env,
		);
		session.start(session.initialEdit);
		const latest = withClip("clip-1");
		commit(session, latest);
		storage.values.clear();
		session.externalChange();
		expect(restoredComposition(storage.access)).toEqual(serializeEdit(latest));
	});

	test("users, guilds and source clips keep separate drafts", () => {
		const storage = sharedStorage();
		const session = new DraftPersistence(
			IDENTITY,
			tab(storage.access, "a").env,
		);
		session.start(session.initialEdit);
		commit(session, withClip("clip-1"));
		for (const other of [
			{ ...IDENTITY, userId: "user-2" },
			{ ...IDENTITY, guildId: "7" },
			{ ...IDENTITY, sourceClipId: "clip-8" },
			{ ...IDENTITY, sourceClipId: null },
		]) {
			const elsewhere = new DraftPersistence(
				other,
				tab(storage.access, "b").env,
			);
			expect(elsewhere.restored).toBe(false);
		}
	});

	test("offers a differing legacy draft without ever rewriting it", () => {
		const legacyKey = legacyDraftKey(IDENTITY.guildId, IDENTITY.sourceClipId);
		const legacyEdit = withClip("clip-9", 0.5);
		const legacyText = JSON.stringify(serializeEdit(legacyEdit));
		const storage = sharedStorage({ [legacyKey]: legacyText });
		const session = new DraftPersistence(
			IDENTITY,
			tab(storage.access, "a").env,
		);
		expect(session.restored).toBe(false);
		const seed = withClip("clip-9");
		commit(session, seed);
		session.start(seed);
		expect(session.getSnapshot().legacyDraftOffered).toBe(true);

		const taken = session.takeLegacyDraft();
		expect(taken && serializeEdit(taken)).toEqual(serializeEdit(legacyEdit));
		expect(session.getSnapshot().legacyDraftOffered).toBe(false);
		if (taken) commit(session, taken);
		expect(restoredComposition(storage.access)).toEqual(
			serializeEdit(legacyEdit),
		);
		expect(storage.values.get(legacyKey)).toBe(legacyText);
	});

	test("does not offer a legacy draft that matches what the editor opened", () => {
		const seed = withClip("clip-9");
		const storage = sharedStorage({
			[legacyDraftKey(IDENTITY.guildId, IDENTITY.sourceClipId)]: JSON.stringify(
				serializeEdit(seed),
			),
		});
		const session = new DraftPersistence(
			IDENTITY,
			tab(storage.access, "a").env,
		);
		commit(session, withClip("clip-9"));
		session.start(withClip("clip-9"));
		expect(session.getSnapshot().legacyDraftOffered).toBe(false);
	});

	test("disposing flushes and leaves the session usable for Strict Mode", () => {
		const storage = sharedStorage();
		const clock = tab(storage.access, "a");
		const session = new DraftPersistence(IDENTITY, clock.env);
		session.start(session.initialEdit);
		const dragging = withClip("clip-1");
		session.update(dragging, session.initialEdit);
		session.dispose();
		expect(clock.pendingTimers()).toBe(0);
		expect(restoredComposition(storage.access)).toEqual(
			serializeEdit(dragging),
		);
		const next = withClip("clip-2");
		commit(session, next);
		expect(restoredComposition(storage.access)).toEqual(serializeEdit(next));
	});
});
