import { describe, expect, test } from "bun:test";
import type { StorageAccess, StorageLike } from "./browserStorage";
import { serializeEdit } from "./composePayload";
import {
	type DraftIdentity,
	draftFile,
	draftKey,
	legacyDraftKey,
	loadDraft,
	loadLegacyDraft,
	parseDraftFile,
	saveDraft,
} from "./draftStorage";
import {
	type ClipEdit,
	DEFAULT_EFFECTS,
	emptyEdit,
	makeSegment,
	type TimelineSegment,
} from "./model";

const IDENTITY: DraftIdentity = {
	userId: "user-1",
	guildId: "42",
	sourceClipId: "clip-9",
};

function memory(initial: Record<string, string> = {}) {
	const values = new Map(Object.entries(initial));
	const storage: StorageLike = {
		getItem: (key) => values.get(key) ?? null,
		setItem: (key, value) => {
			values.set(key, value);
		},
	};
	const access: StorageAccess = { ok: true, storage };
	return { values, access };
}

function failingWrites(error: unknown): StorageAccess {
	return {
		ok: true,
		storage: {
			getItem: () => null,
			setItem: () => {
				throw error;
			},
		},
	};
}

function segment(
	id: string,
	sourceIn: number,
	sourceOut: number,
): TimelineSegment {
	return makeSegment("clip", id, sourceIn, sourceOut, 0, 0);
}

function edit(...segments: TimelineSegment[]): ClipEdit {
	return {
		segments,
		tracks: 2,
		mutedTracks: [false, false],
		masterVolumeDb: -3,
	};
}

const writeOptions = { expectedRaw: null, writer: "tab-a", savedAt: 1_000 };

describe("draftKey", () => {
	test("scopes drafts by user, guild and source clip", () => {
		expect(draftKey(IDENTITY)).toBe(
			"sakiot:clip-editor:drafts:v1:user-1:42:clip:clip-9",
		);
		expect(draftKey({ ...IDENTITY, sourceClipId: null })).toBe(
			"sakiot:clip-editor:drafts:v1:user-1:42:generic",
		);
		const keys = new Set([
			draftKey(IDENTITY),
			draftKey({ ...IDENTITY, userId: "user-2" }),
			draftKey({ ...IDENTITY, guildId: "7" }),
			draftKey({ ...IDENTITY, sourceClipId: "clip-8" }),
			draftKey({ ...IDENTITY, sourceClipId: null }),
		]);
		expect(keys.size).toBe(5);
	});

	test("never collides with the unscoped legacy keys", () => {
		expect(legacyDraftKey("42", "clip-9")).toBe(
			"sakiot:clip-editor:42:clip:clip-9",
		);
		expect(legacyDraftKey("42", null)).toBe("sakiot:clip-editor:42:draft");
		expect(draftKey(IDENTITY)).not.toBe(legacyDraftKey("42", "clip-9"));
	});
});

describe("loadDraft / saveDraft", () => {
	test("round-trips an edit with its revision, writer and save time", () => {
		const { access } = memory();
		const clip = segment("clip-1", 1, 5);
		clip.effects = { ...DEFAULT_EFFECTS, rate: 1.5, reverse: true };
		const source = edit(clip);
		source.mutedTracks[1] = true;
		const saved = saveDraft(IDENTITY, source, writeOptions, access);
		expect(saved.status).toBe("saved");
		const loaded = loadDraft(IDENTITY, access);
		if (loaded.status !== "loaded") throw new Error(loaded.status);
		expect(loaded.draft).toMatchObject({
			revision: 1,
			writer: "tab-a",
			savedAt: 1_000,
		});
		expect(loaded.draft.edit.masterVolumeDb).toBe(-3);
		expect(loaded.draft.edit.mutedTracks).toEqual([false, true]);
		expect(loaded.draft.edit.segments[0]).toMatchObject({
			sourceId: "clip-1",
			sourceIn: 1,
			sourceOut: 5,
		});
		expect(loaded.draft.edit.segments[0]?.effects).toMatchObject({
			rate: 1.5,
			reverse: true,
		});
	});

	test("tells a missing draft apart from a saved empty edit", () => {
		const { access } = memory();
		expect(loadDraft(IDENTITY, access)).toEqual({ status: "missing" });
		saveDraft(IDENTITY, emptyEdit(), writeOptions, access);
		const loaded = loadDraft(IDENTITY, access);
		expect(loaded.status).toBe("loaded");
		if (loaded.status === "loaded") {
			expect(loaded.draft.edit.segments).toEqual([]);
		}
	});

	test("stores the payload the export would send", () => {
		const { access, values } = memory();
		const source = edit(segment("clip-1", 0, 4));
		saveDraft(IDENTITY, source, writeOptions, access);
		const record = JSON.parse(values.get(draftKey(IDENTITY)) ?? "{}");
		expect(record.version).toBe(1);
		expect(record.composition).toEqual(serializeEdit(source));
	});

	test("increments the revision on every write", () => {
		const { access } = memory();
		const first = saveDraft(IDENTITY, emptyEdit(), writeOptions, access);
		if (first.status !== "saved") throw new Error(first.status);
		const second = saveDraft(
			IDENTITY,
			emptyEdit(),
			{ ...writeOptions, expectedRaw: first.draft.raw },
			access,
		);
		if (second.status !== "saved") throw new Error(second.status);
		expect(second.draft.revision).toBe(2);
	});

	test("reports malformed records instead of treating them as missing", () => {
		for (const raw of [
			"{not json",
			"[]",
			JSON.stringify({ version: 2, composition: serializeEdit(emptyEdit()) }),
			JSON.stringify({
				version: 1,
				revision: 1,
				writer: "tab-a",
				savedAt: 1,
				composition: { segments: "nope" },
			}),
			JSON.stringify({
				version: 1,
				revision: 0,
				writer: "tab-a",
				savedAt: 1,
				composition: serializeEdit(emptyEdit()),
			}),
		]) {
			const { access } = memory({ [draftKey(IDENTITY)]: raw });
			expect(loadDraft(IDENTITY, access)).toEqual({ status: "malformed", raw });
		}
	});

	test("reports unavailable storage, including a throwing read", () => {
		expect(
			loadDraft(IDENTITY, { ok: false, error: new Error("blocked") }),
		).toEqual({ status: "unavailable" });
		const throwing: StorageAccess = {
			ok: true,
			storage: {
				getItem: () => {
					throw new DOMException("denied", "SecurityError");
				},
				setItem: () => {},
			},
		};
		expect(loadDraft(IDENTITY, throwing)).toEqual({ status: "unavailable" });
	});

	test("classifies write failures", () => {
		const source = edit(segment("clip-1", 0, 4));
		expect(
			saveDraft(
				IDENTITY,
				source,
				writeOptions,
				failingWrites(new DOMException("full", "QuotaExceededError")),
			),
		).toEqual({ status: "error", reason: "quota" });
		expect(
			saveDraft(
				IDENTITY,
				source,
				writeOptions,
				failingWrites(new DOMException("denied", "SecurityError")),
			),
		).toEqual({ status: "error", reason: "unavailable" });
		expect(
			saveDraft(IDENTITY, source, writeOptions, failingWrites(new Error("?"))),
		).toEqual({ status: "error", reason: "failed" });
		expect(
			saveDraft(IDENTITY, source, writeOptions, {
				ok: false,
				error: new Error("blocked"),
			}),
		).toEqual({ status: "error", reason: "unavailable" });
	});

	test("refuses to overwrite text it has not seen", () => {
		const { access, values } = memory();
		const theirs = saveDraft(
			IDENTITY,
			edit(segment("theirs", 0, 1)),
			{ ...writeOptions, writer: "tab-b" },
			access,
		);
		if (theirs.status !== "saved") throw new Error(theirs.status);
		const mine = saveDraft(
			IDENTITY,
			edit(segment("mine", 0, 1)),
			writeOptions,
			access,
		);
		expect(mine.status).toBe("conflict");
		if (mine.status === "conflict") {
			expect(mine.current.status).toBe("loaded");
		}
		expect(values.get(draftKey(IDENTITY))).toBe(theirs.draft.raw);
	});

	test("writes over a key that vanished since it was read", () => {
		const { access } = memory();
		const result = saveDraft(
			IDENTITY,
			emptyEdit(),
			{ ...writeOptions, expectedRaw: "cleared since" },
			access,
		);
		expect(result.status).toBe("saved");
	});
});

describe("loadLegacyDraft", () => {
	test("reads the bare composition payload from the unscoped key", () => {
		const { access } = memory({
			[legacyDraftKey("42", null)]: JSON.stringify(
				serializeEdit(edit(segment("clip-1", 0, 2))),
			),
		});
		expect(loadLegacyDraft("42", null, access)?.segments).toHaveLength(1);
		expect(loadLegacyDraft("42", "clip-9", access)).toBeNull();
	});

	test("ignores unreadable legacy data", () => {
		const { access } = memory({ [legacyDraftKey("42", null)]: "{not json" });
		expect(loadLegacyDraft("42", null, access)).toBeNull();
	});
});

describe("draft files", () => {
	test("a downloaded draft opens again in the same server", () => {
		const source = edit(segment("clip-1", 0.5, 2));
		const file = draftFile(IDENTITY, source, Date.UTC(2026, 8, 30, 12));
		expect(file.name).toBe(
			"sakiot-clip-draft-clip-9-2026-09-30T12-00-00-000Z.json",
		);
		const opened = parseDraftFile(file.text, "42");
		if (!opened.ok) throw new Error(opened.message);
		expect(serializeEdit(opened.edit)).toEqual(serializeEdit(source));
	});

	test("refuses files from another server, other files and newer versions", () => {
		const file = draftFile(IDENTITY, emptyEdit(), 0);
		expect(parseDraftFile(file.text, "7")).toMatchObject({ ok: false });
		expect(parseDraftFile("{}", "42")).toEqual({
			ok: false,
			message: "This file isn't a clip editor draft.",
		});
		expect(parseDraftFile("not json", "42")).toMatchObject({ ok: false });
		const newer = { ...JSON.parse(file.text), version: 2 };
		expect(parseDraftFile(JSON.stringify(newer), "42")).toEqual({
			ok: false,
			message: "This draft was saved by a newer version of the editor.",
		});
		const damaged = { ...JSON.parse(file.text), composition: {} };
		expect(parseDraftFile(JSON.stringify(damaged), "42")).toEqual({
			ok: false,
			message: "This draft file is damaged.",
		});
	});
});
