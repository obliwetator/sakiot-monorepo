import { describe, expect, test } from "bun:test";
import { deserializeEdit, serializeEdit } from "./composePayload";
import {
	type ClipEdit,
	DEFAULT_EFFECTS,
	makeSegment,
	type TimelineSegment,
} from "./model";

function segment(
	id: string,
	track: number,
	start: number,
	sourceIn: number,
	sourceOut: number,
): TimelineSegment {
	return makeSegment("clip", id, sourceIn, sourceOut, start, track);
}

function edit(...segments: TimelineSegment[]): ClipEdit {
	return { segments, tracks: 2, masterVolumeDb: -3 };
}

describe("serializeEdit", () => {
	test("maps every segment field to snake_case", () => {
		const payload = serializeEdit(
			edit(segment("clip-1", 0, 2.5, 1, 5)),
			"my composition",
		);
		expect(payload.name).toBe("my composition");
		expect(payload.master_volume_db).toBe(-3);
		expect(payload.segments).toHaveLength(1);
		expect(payload.segments[0]).toEqual({
			source: "clip",
			source_id: "clip-1",
			source_in: 1,
			source_out: 5,
			timeline_start: 2.5,
			track: 0,
			effects: {
				volume_db: 0,
				pitch_cents: 0,
				rate: 1,
				bass_db: 0,
				treble_db: 0,
				reverse: false,
			},
		});
	});

	test("preserves non-default effects", () => {
		const base = segment("clip-1", 0, 0, 0, 4);
		base.effects = {
			volumeDb: -6,
			pitchCents: 300,
			rate: 1.5,
			bassDb: 3,
			trebleDb: -3,
			reverse: true,
		};
		const payload = serializeEdit(edit(base));
		expect(payload.segments[0].effects).toEqual({
			volume_db: -6,
			pitch_cents: 300,
			rate: 1.5,
			bass_db: 3,
			treble_db: -3,
			reverse: true,
		});
	});

	test("omits the name when not provided", () => {
		const payload = serializeEdit(edit(segment("clip-1", 0, 0, 0, 1)));
		expect(payload.name).toBeUndefined();
		expect(payload.segments).toHaveLength(1);
	});

	test("serializes every segment in order", () => {
		const payload = serializeEdit(
			edit(
				segment("a", 0, 0, 0, 2),
				segment("b", 1, 3, 0, 2),
				segment("a", 0, 5, 2, 4),
			),
		);
		expect(payload.segments.map((s) => s.source_id)).toEqual(["a", "b", "a"]);
		expect(payload.segments.map((s) => s.timeline_start)).toEqual([0, 3, 5]);
		expect(payload.segments.map((s) => s.track)).toEqual([0, 1, 0]);
	});

	test("defaults to DEFAULT_EFFECTS values", () => {
		expect(DEFAULT_EFFECTS).toEqual({
			volumeDb: 0,
			pitchCents: 0,
			rate: 1,
			bassDb: 0,
			trebleDb: 0,
			reverse: false,
		});
	});
});

function segmentPayload() {
	return {
		source: "clip",
		source_id: "clip-1",
		source_in: 0,
		source_out: 4,
		timeline_start: 0,
		track: 0,
		effects: {
			volume_db: 0,
			pitch_cents: 0,
			rate: 1,
			bass_db: 0,
			treble_db: 0,
		},
	};
}

describe("deserializeEdit", () => {
	test("round-trips a serializeEdit payload", () => {
		const sourceEdit = edit(
			segment("clip-1", 0, 2.5, 1, 5),
			segment("clip-2", 1, 8, 0, 3),
		);
		sourceEdit.segments[1].effects = {
			volumeDb: -6,
			pitchCents: 300,
			rate: 1.5,
			bassDb: 3,
			trebleDb: -3,
			reverse: true,
		};
		const restored = deserializeEdit(serializeEdit(sourceEdit, "remix"));
		expect(restored).not.toBeNull();
		expect(restored?.masterVolumeDb).toBe(-3);
		expect(restored?.tracks).toBe(2);
		expect(restored?.segments).toHaveLength(2);
		expect(restored?.segments[0]).toMatchObject({
			track: 0,
			source: "clip",
			sourceId: "clip-1",
			sourceIn: 1,
			sourceOut: 5,
			timelineStart: 2.5,
		});
		expect(restored?.segments[1].effects).toEqual({
			volumeDb: -6,
			pitchCents: 300,
			rate: 1.5,
			bassDb: 3,
			trebleDb: -3,
			reverse: true,
		});
		expect(restored?.segments[0].id).not.toBe(restored?.segments[1].id);
	});

	test("compositions without the reverse flag deserialize forward", () => {
		const restored = deserializeEdit({
			segments: [segmentPayload()],
			master_volume_db: 0,
		});
		expect(restored).not.toBeNull();
		expect(restored?.segments[0].effects.reverse).toBe(false);
	});

	test("rejects a non-boolean reverse flag", () => {
		const payload = {
			segments: [
				{
					...segmentPayload(),
					effects: { ...segmentPayload().effects, reverse: "yes" },
				},
			],
			master_volume_db: 0,
		};
		expect(deserializeEdit(payload)).toBeNull();
	});

	test("rejects missing or malformed payloads", () => {
		expect(deserializeEdit(null)).toBeNull();
		expect(deserializeEdit(undefined)).toBeNull();
		expect(deserializeEdit("nope")).toBeNull();
		expect(deserializeEdit({})).toBeNull();
		expect(deserializeEdit({ segments: "x" })).toBeNull();
		expect(
			deserializeEdit({ segments: [{ source: "clip" }], master_volume_db: 0 }),
		).toBeNull();
		expect(
			deserializeEdit({
				segments: [segmentPayload()],
				master_volume_db: Number.NaN,
			}),
		).toBeNull();
	});

	test("rejects unknown source types", () => {
		const payload = {
			segments: [{ ...segmentPayload(), source: "livestream" }],
			master_volume_db: 0,
		};
		expect(deserializeEdit(payload)).toBeNull();
	});

	test("rejects segments with non-finite effects", () => {
		const payload = {
			segments: [
				{
					...segmentPayload(),
					effects: { ...segmentPayload().effects, rate: Number.NaN },
				},
			],
			master_volume_db: 0,
		};
		expect(deserializeEdit(payload)).toBeNull();
	});
});
