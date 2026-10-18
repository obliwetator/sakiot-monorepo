import type { Removal } from "../../app/apiSlice";
import type { Channels } from "../../Constants";

/**
 * Why the recording tree lists one of the viewer's own sessions although
 * everyone else no longer sees it, or null when it lists the session as
 * visible, or not at all.
 */
export function sessionRemoval(
	tree: Channels[] | undefined,
	sessionId: string,
): Removal | null {
	for (const channel of tree ?? []) {
		for (const dir of channel.dirs) {
			for (const files of Object.values(dir.months ?? {})) {
				for (const file of files ?? []) {
					if (file.recording_session_id === sessionId) {
						return file.removal ?? null;
					}
				}
			}
		}
	}
	return null;
}
