import type { components } from "../../api/openapi";
import type { ClipEdit, SegmentEffects, TimelineSegment } from "./model";
import { newSegmentId } from "./model";

type ComposeClipBody = components["schemas"]["ComposeClipBody"];

/**
 * Mirrors the ClipEdit state onto the server's snake_case composition
 * request. The export endpoint renders exactly what the Web Audio engine
 * previews: per-segment trim, volume, pitch, rate, bass, and treble, mixed
 * onto the timeline with the master volume applied to the result.
 */
export function serializeEdit(edit: ClipEdit, name?: string): ComposeClipBody {
	return {
		name,
		master_volume_db: edit.masterVolumeDb,
		segments: edit.segments.map((segment) => ({
			source: segment.source,
			source_id: segment.sourceId,
			source_in: segment.sourceIn,
			source_out: segment.sourceOut,
			timeline_start: segment.timelineStart,
			track: segment.track,
			merge_group: segment.mergeGroup,
			effects: {
				volume_db: segment.effects.volumeDb,
				pitch_cents: segment.effects.pitchCents,
				rate: segment.effects.rate,
				bass_db: segment.effects.bassDb,
				mid_db: segment.effects.midDb,
				treble_db: segment.effects.trebleDb,
				reverse: segment.effects.reverse,
			},
		})),
	};
}

/**
 * Inverse of serializeEdit: rebuilds a ClipEdit from the composition JSON a
 * composed clip carries in the database. Returns null when the payload is
 * missing or malformed so callers can fall back to importing the clip as a
 * single segment.
 */
export function deserializeEdit(data: unknown): ClipEdit | null {
	if (typeof data !== "object" || data === null) return null;
	const record = data as Record<string, unknown>;
	const rawSegments = record.segments;
	if (!Array.isArray(rawSegments)) return null;
	const segments: TimelineSegment[] = [];
	for (const raw of rawSegments) {
		const segment = deserializeSegment(raw);
		if (!segment) return null;
		segments.push(segment);
	}
	const masterVolumeDb = record.master_volume_db;
	if (typeof masterVolumeDb !== "number" || !Number.isFinite(masterVolumeDb)) {
		return null;
	}
	const tracks = segments.reduce(
		(max, segment) => Math.max(max, segment.track + 1),
		1,
	);
	return { segments, tracks, masterVolumeDb };
}

function deserializeSegment(raw: unknown): TimelineSegment | null {
	if (typeof raw !== "object" || raw === null) return null;
	const segment = raw as Record<string, unknown>;
	if (
		segment.source !== "clip" &&
		segment.source !== "session" &&
		segment.source !== "session-silence-free"
	) {
		return null;
	}
	const {
		source_id: sourceId,
		source_in: sourceIn,
		source_out: sourceOut,
	} = segment;
	const timelineStart = segment.timeline_start;
	const track = segment.track;
	const mergeGroup = segment.merge_group;
	const effects = deserializeEffects(segment.effects);
	if (typeof sourceId !== "string") return null;
	if (!isFiniteNumber(sourceIn) || !isFiniteNumber(sourceOut)) return null;
	if (!isFiniteNumber(timelineStart) || timelineStart < 0) return null;
	if (typeof track !== "number" || !Number.isInteger(track) || track < 0) {
		return null;
	}
	if (mergeGroup !== undefined && mergeGroup !== null) {
		if (typeof mergeGroup !== "string" || mergeGroup.length === 0) return null;
	}
	if (!effects) return null;
	return {
		id: newSegmentId(),
		track,
		source: segment.source as TimelineSegment["source"],
		sourceId,
		sourceIn,
		sourceOut,
		timelineStart,
		effects,
		...(typeof mergeGroup === "string" ? { mergeGroup } : {}),
	};
}

function deserializeEffects(raw: unknown): SegmentEffects | null {
	if (typeof raw !== "object" || raw === null) return null;
	const effects = raw as Record<string, unknown>;
	const { volume_db: volumeDb, pitch_cents: pitchCents } = effects;
	const {
		rate,
		bass_db: bassDb,
		mid_db: midDb = 0,
		treble_db: trebleDb,
	} = effects;
	const reverse = effects.reverse;
	if (
		!isFiniteNumber(volumeDb) ||
		!isFiniteNumber(pitchCents) ||
		!isFiniteNumber(rate) ||
		!isFiniteNumber(bassDb) ||
		!isFiniteNumber(midDb) ||
		!isFiniteNumber(trebleDb)
	) {
		return null;
	}
	// Compositions exported before reverse existed omit the flag; treat the
	// absence as forward playback.
	if (reverse !== undefined && typeof reverse !== "boolean") return null;
	return {
		volumeDb,
		pitchCents,
		rate,
		bassDb,
		midDb,
		trebleDb,
		reverse: reverse === true,
	};
}

function isFiniteNumber(value: unknown): value is number {
	return typeof value === "number" && Number.isFinite(value);
}
