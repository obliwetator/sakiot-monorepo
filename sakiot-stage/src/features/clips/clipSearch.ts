import type { ClipData } from "../../app/apiSlice";

/**
 * Case-insensitive substring match over what a person can see or paste: the
 * clip name, the clip id from the URL, the owning user id, and the recording
 * the clip was cut from. An empty query returns the list unchanged.
 */
export function filterClips(clips: ClipData[], query: string): ClipData[] {
	const needle = query.trim().toLowerCase();
	if (!needle) return clips;

	return clips.filter((clip) =>
		[clip.name, clip.clip_id, clip.user_id, clip.original_file_name].some(
			(value) =>
				typeof value === "string" && value.toLowerCase().includes(needle),
		),
	);
}
