import type { Removal } from "../../app/apiSlice";
import type { Channels } from "../../Constants";

type TreeFile = NonNullable<
	NonNullable<Channels["dirs"][number]["months"]>[number]
>[number];

/** The recording tree's entry for a session, or null when it lists none. */
function sessionEntry(
	tree: Channels[] | undefined,
	sessionId: string,
): TreeFile | null {
	for (const channel of tree ?? []) {
		for (const dir of channel.dirs) {
			for (const files of Object.values(dir.months ?? {})) {
				for (const file of files ?? []) {
					if (file.recording_session_id === sessionId) return file;
				}
			}
		}
	}
	return null;
}

/**
 * Why the recording tree lists one of the viewer's own sessions although
 * everyone else no longer sees it, or null when it lists the session as
 * visible, or not at all.
 */
export function sessionRemoval(
	tree: Channels[] | undefined,
	sessionId: string,
): Removal | null {
	return sessionEntry(tree, sessionId)?.removal ?? null;
}

/**
 * Whether the tree lists the session redacted: its speaker keeps it
 * private, and the viewer sees it only to remove it.
 */
export function sessionRedacted(
	tree: Channels[] | undefined,
	sessionId: string,
): boolean {
	return sessionEntry(tree, sessionId)?.redacted === true;
}
