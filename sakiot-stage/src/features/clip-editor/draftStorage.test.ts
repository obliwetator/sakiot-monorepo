import { describe, expect, test } from "bun:test";
import { serializeEdit } from "./composePayload";
import {
	draftKey,
	loadDraft,
	type StorageLike,
	saveDraft,
} from "./draftStorage";
import {
	type ClipEdit,
	DEFAULT_EFFECTS,
	makeSegment,
	type TimelineSegment,
} from "./model";

function storage(initial: Record<string, string> = {}): StorageLike {
	const values = new Map(Object.entries(initial));
	return {
		getItem: (key) => values.get(key) ?? null,
		setItem: (key, value) => {
			values.set(key, value);
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

describe("draftKey", () => {
	test("uses the source clip id when editing a composed clip", () => {
		expect(draftKey("42", "clip-9")).toBe("sakiot:clip-editor:42:clip:clip-9");
	});

	test("shares one draft for the generic editor", () => {
		expect(draftKey("42", null)).toBe("sakiot:clip-editor:42:draft");
	});

	test("keeps clips and guilds apart", () => {
		expect(draftKey("42", "clip-9")).not.toBe(draftKey("42", "clip-8"));
		expect(draftKey("42", null)).not.toBe(draftKey("7", null));
	});
});

describe("saveDraft / loadDraft", () => {
	test("round-trips an edit through the composition payload", () => {
		const store = storage();
		const source = edit(segment("clip-1", 1, 5));
		source.mutedTracks[1] = true;
		source.segments[0].effects = {
			...DEFAULT_EFFECTS,
			rate: 1.5,
			reverse: true,
		};
		saveDraft("42", "clip-9", source, store);
		const restored = loadDraft("42", "clip-9", store);
		expect(restored).not.toBeNull();
		expect(restored?.masterVolumeDb).toBe(-3);
		expect(restored?.mutedTracks).toEqual([false, true]);
		expect(restored?.segments).toHaveLength(1);
		expect(restored?.segments[0]).toMatchObject({
			sourceId: "clip-1",
			sourceIn: 1,
			sourceOut: 5,
		});
		expect(restored?.segments[0].effects).toMatchObject({
			rate: 1.5,
			reverse: true,
		});
	});

	test("restores nothing when no draft exists", () => {
		expect(loadDraft("42", "clip-9", storage())).toBeNull();
	});

	test("restores only the matching session context", () => {
		const store = storage();
		saveDraft("42", "clip-9", edit(segment("clip-1", 0, 4)), store);
		expect(loadDraft("42", null, store)).toBeNull();
		expect(loadDraft("7", "clip-9", store)).toBeNull();
		expect(loadDraft("42", "clip-9", store)).not.toBeNull();
	});

	test("treats corrupted payloads as missing", () => {
		const store = storage({ "sakiot:clip-editor:42:draft": "{not json" });
		expect(loadDraft("42", null, store)).toBeNull();
	});

	test("swallows storage write failures", () => {
		const broken: StorageLike = {
			getItem: () => null,
			setItem: () => {
				throw new Error("quota exceeded");
			},
		};
		expect(() =>
			saveDraft("42", null, edit(segment("clip-1", 0, 4)), broken),
		).not.toThrow();
	});

	test("no-ops without a storage backend", () => {
		expect(loadDraft("42", null, null)).toBeNull();
		expect(() =>
			saveDraft("42", null, edit(segment("clip-1", 0, 4)), null),
		).not.toThrow();
	});

	test("a draft is the same payload the export would send", () => {
		const store = storage();
		const source = edit(segment("clip-1", 0, 4));
		saveDraft("42", null, source, store);
		const raw = store.getItem(draftKey("42", null)) ?? "{}";
		expect(JSON.parse(raw)).toEqual(serializeEdit(source));
	});
});
