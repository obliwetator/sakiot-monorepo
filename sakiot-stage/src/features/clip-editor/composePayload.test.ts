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

function serializedAdvanced(effects = DEFAULT_EFFECTS) {
	return {
		tail_seconds: effects.tailSeconds,
		distortion_amount: effects.distortionAmount,
		distortion_wet: effects.distortionWet,
		delay_seconds: effects.delaySeconds,
		delay_feedback: effects.delayFeedback,
		delay_wet: effects.delayWet,
		compressor_enabled: effects.compressorEnabled,
		compressor_threshold_db: effects.compressorThresholdDb,
		compressor_knee_db: effects.compressorKneeDb,
		compressor_ratio: effects.compressorRatio,
		compressor_attack_seconds: effects.compressorAttackSeconds,
		compressor_release_seconds: effects.compressorReleaseSeconds,
		chorus_enabled: effects.chorusEnabled,
		chorus_frequency_hz: effects.chorusFrequencyHz,
		chorus_delay_ms: effects.chorusDelayMs,
		chorus_depth: effects.chorusDepth,
		chorus_spread_degrees: effects.chorusSpreadDegrees,
		chorus_feedback: effects.chorusFeedback,
		chorus_wet: effects.chorusWet,
		reverb_enabled: effects.reverbEnabled,
		reverb_decay_seconds: effects.reverbDecaySeconds,
		reverb_pre_delay_seconds: effects.reverbPreDelaySeconds,
		reverb_wet: effects.reverbWet,
		reverb_seed: effects.reverbSeed,
	};
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
				mid_db: 0,
				treble_db: 0,
				advanced: serializedAdvanced(),
				reverse: false,
			},
		});
	});

	test("preserves non-default effects", () => {
		const base = segment("clip-1", 0, 0, 0, 4);
		base.effects = {
			...DEFAULT_EFFECTS,
			volumeDb: -6,
			pitchCents: 300,
			rate: 1.5,
			bassDb: 3,
			midDb: 1.5,
			trebleDb: -3,
			distortionAmount: 0.8,
			distortionWet: 0.6,
			delaySeconds: 0.4,
			delayFeedback: 0.35,
			delayWet: 0.25,
			compressorEnabled: true,
			chorusEnabled: true,
			reverbEnabled: true,
			reverbSeed: 42,
			reverse: true,
		};
		const payload = serializeEdit(edit(base));
		expect(payload.segments[0].effects).toEqual({
			volume_db: -6,
			pitch_cents: 300,
			rate: 1.5,
			bass_db: 3,
			mid_db: 1.5,
			treble_db: -3,
			advanced: serializedAdvanced(base.effects),
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

	test("serializes the merge group of merged units", () => {
		const first = segment("clip-1", 0, 0, 0, 4);
		const second = segment("clip-2", 0, 4, 4, 8);
		first.mergeGroup = "group-7";
		second.mergeGroup = "group-7";
		const payload = serializeEdit(edit(first, second));
		expect(payload.segments.map((s) => s.merge_group)).toEqual([
			"group-7",
			"group-7",
		]);
	});

	test("defaults to DEFAULT_EFFECTS values", () => {
		expect(DEFAULT_EFFECTS).toEqual({
			volumeDb: 0,
			pitchCents: 0,
			rate: 1,
			tailSeconds: 0,
			bassDb: 0,
			midDb: 0,
			trebleDb: 0,
			distortionAmount: 0.4,
			distortionWet: 0,
			delaySeconds: 0.25,
			delayFeedback: 0.125,
			delayWet: 0,
			compressorEnabled: false,
			compressorThresholdDb: -24,
			compressorKneeDb: 30,
			compressorRatio: 12,
			compressorAttackSeconds: 0.003,
			compressorReleaseSeconds: 0.25,
			chorusEnabled: false,
			chorusFrequencyHz: 1.5,
			chorusDelayMs: 3.5,
			chorusDepth: 0.7,
			chorusSpreadDegrees: 180,
			chorusFeedback: 0,
			chorusWet: 0.5,
			reverbEnabled: false,
			reverbDecaySeconds: 1.5,
			reverbPreDelaySeconds: 0.01,
			reverbWet: 1,
			reverbSeed: 0x5341_4b49,
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
			mid_db: 0,
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
			...DEFAULT_EFFECTS,
			volumeDb: -6,
			tailSeconds: 2,
			pitchCents: 300,
			rate: 1.5,
			bassDb: 3,
			midDb: 1.5,
			trebleDb: -3,
			distortionAmount: 0.8,
			distortionWet: 0.6,
			delayWet: 0.25,
			compressorEnabled: true,
			chorusEnabled: true,
			reverbEnabled: true,
			reverbSeed: 42,
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
		expect(restored?.segments[1].effects).toMatchObject({
			volumeDb: -6,
			tailSeconds: 2,
			pitchCents: 300,
			rate: 1.5,
			bassDb: 3,
			midDb: 1.5,
			trebleDb: -3,
			distortionAmount: 0.8,
			distortionWet: 0.6,
			delayWet: 0.25,
			compressorEnabled: true,
			chorusEnabled: true,
			reverbEnabled: true,
			reverbSeed: 42,
			reverse: true,
		});
		expect(restored?.segments[0].id).not.toBe(restored?.segments[1].id);
	});

	test("round-trips merged units so they re-import grouped", () => {
		const first = segment("clip-1", 0, 0, 0, 4);
		const second = segment("clip-2", 0, 4, 4, 8);
		first.mergeGroup = "group-7";
		second.mergeGroup = "group-7";
		const restored = deserializeEdit(serializeEdit(edit(first, second)));
		expect(restored).not.toBeNull();
		expect(restored?.segments[0].mergeGroup).toBe("group-7");
		expect(restored?.segments[1].mergeGroup).toBe("group-7");
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

	test("compositions without mid EQ deserialize flat", () => {
		const payload = segmentPayload();
		const { mid_db: _midDb, ...legacyEffects } = payload.effects;
		const restored = deserializeEdit({
			segments: [{ ...payload, effects: legacyEffects }],
			master_volume_db: 0,
		});
		expect(restored).not.toBeNull();
		expect(restored?.segments[0].effects.midDb).toBe(0);
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

	test("compositions without a merge group deserialize ungrouped", () => {
		const restored = deserializeEdit({
			segments: [segmentPayload()],
			master_volume_db: 0,
		});
		expect(restored).not.toBeNull();
		expect(restored?.segments[0].mergeGroup).toBeUndefined();
	});

	test("rejects an empty merge group id", () => {
		const payload = {
			segments: [{ ...segmentPayload(), merge_group: "" }],
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
