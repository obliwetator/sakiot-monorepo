import type { components } from "../api/openapi";

type Channels = components["schemas"]["Channels"];
type Directories = components["schemas"]["Directories"];
type File = components["schemas"]["File"];

interface Location {
	channelId: string;
	year: number;
	month: string;
	index: number;
}

function filesOf(dir: Directories, month: string): File[] | null | undefined {
	return dir.months?.[month] as File[] | null | undefined;
}

/** Removes the session from the tree; returns where it was. */
function removeSession(tree: Channels[], sessionId: string): Location | null {
	for (const channel of tree) {
		for (const dir of channel.dirs) {
			for (const month of Object.keys(dir.months ?? {})) {
				const files = filesOf(dir, month);
				const index =
					files?.findIndex((file) => file.recording_session_id === sessionId) ??
					-1;
				if (files && index >= 0) {
					files.splice(index, 1);
					return {
						channelId: channel.channel_id,
						year: dir.year,
						month,
						index,
					};
				}
			}
		}
	}
	return null;
}

/** Drops months, years and channels left without any file. */
function prune(tree: Channels[]): void {
	for (let c = tree.length - 1; c >= 0; c -= 1) {
		const channel = tree[c];
		if (!channel) continue;
		for (let d = channel.dirs.length - 1; d >= 0; d -= 1) {
			const dir = channel.dirs[d];
			if (!dir) continue;
			for (const month of Object.keys(dir.months ?? {})) {
				if ((filesOf(dir, month)?.length ?? 0) === 0 && dir.months) {
					delete dir.months[month];
				}
			}
			if (Object.keys(dir.months ?? {}).length === 0) channel.dirs.splice(d, 1);
		}
		if (channel.dirs.length === 0) tree.splice(c, 1);
	}
}

/** Discord ids exceed 2^53: compare them as integers, not as numbers. */
function compareIds(a: string, b: string): number {
	const left = BigInt(a);
	const right = BigInt(b);
	return left < right ? -1 : left > right ? 1 : 0;
}

/**
 * Applies one session's fresh tree entry (from the one-session endpoint) to a
 * cached recording tree, in place (an Immer draft). `entry` null removes the
 * session: it was deleted or is no longer visible.
 *
 * A session's starting channel and start time never change, so it stays in
 * the same group; within a month an updated session keeps its position and a
 * new one goes first, matching the listing's newest-first order. Empty groups
 * are pruned; channels stay sorted by id and years newest first.
 */
export function patchSession(
	tree: Channels[],
	sessionId: string,
	entry: Channels | null,
): void {
	const previous = removeSession(tree, sessionId);
	const dir = entry?.dirs[0];
	const month = dir ? Object.keys(dir.months ?? {})[0] : undefined;
	const file = dir && month ? filesOf(dir, month)?.[0] : undefined;
	if (!entry || !dir || month === undefined || !file) {
		prune(tree);
		return;
	}

	let channel = tree.find((c) => c.channel_id === entry.channel_id);
	if (!channel) {
		channel = { channel_id: entry.channel_id, dirs: [] };
		const at = tree.findIndex(
			(c) => compareIds(c.channel_id, entry.channel_id) > 0,
		);
		tree.splice(at < 0 ? tree.length : at, 0, channel);
	}
	let targetDir = channel.dirs.find((d) => d.year === dir.year);
	if (!targetDir) {
		targetDir = { year: dir.year, months: {} };
		const at = channel.dirs.findIndex((d) => d.year < dir.year);
		channel.dirs.splice(at < 0 ? channel.dirs.length : at, 0, targetDir);
	}
	targetDir.months ??= {};
	const files = filesOf(targetDir, month) ?? [];
	const samePlace =
		previous &&
		previous.channelId === entry.channel_id &&
		previous.year === dir.year &&
		previous.month === month;
	files.splice(samePlace ? Math.min(previous.index, files.length) : 0, 0, file);
	targetDir.months[month] = files;
	prune(tree);
}
