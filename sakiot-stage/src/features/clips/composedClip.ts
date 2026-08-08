import type { ClipData } from "../../app/apiSlice";

/** original_file_name marker written by the clip editor export endpoint. */
export const COMPOSED_SOURCE = "compose";

export function isComposedClip(clip: ClipData): boolean {
	return clip.original_file_name === COMPOSED_SOURCE;
}
