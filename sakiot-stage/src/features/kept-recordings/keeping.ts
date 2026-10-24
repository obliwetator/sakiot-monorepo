import type {
	Keeping,
	KeptRecording,
	RemovedRecording,
} from "../../app/apiSlice";

export function recordings(count: number): string {
	return `${count} recording${count === 1 ? "" : "s"}`;
}

/** When something happened, as a day and a time. */
export function when(ms: number): string {
	return new Date(ms).toLocaleString(undefined, {
		dateStyle: "medium",
		timeStyle: "short",
	});
}

/** How much of the server's cap is kept. */
export function serverUsage(keeping: Pick<Keeping, "cap" | "kept">): string {
	if (keeping.cap === 0 && keeping.kept === 0)
		return "This server can't keep recordings yet: the bot's operators decide how many.";
	return `This server keeps ${keeping.kept} of the ${recordings(keeping.cap)} it may keep.`;
}

/** How much of their allowance the viewer keeps. */
export function viewerUsage(
	keeping: Pick<Keeping, "allowance" | "used">,
): string {
	const { allowance, used } = keeping;
	if (allowance === null || allowance === undefined)
		return used === 0
			? "You may keep recordings until the server's cap is reached."
			: `You keep ${recordings(used)}, and may keep more until the server's cap is reached.`;
	if (allowance === 0 && used === 0) return "You may not keep recordings here.";
	return `You keep ${used} of the ${recordings(allowance)} you may keep.`;
}

/**
 * Why the viewer cannot keep a recording nobody keeps yet, or null when they
 * can. The server decides; this only spares a request bound to fail.
 */
export function keepBlocked(
	keeping: Pick<Keeping, "allowance" | "used" | "cap" | "kept">,
): string | null {
	const { allowance, used, cap, kept } = keeping;
	if (allowance !== null && allowance !== undefined && used >= allowance)
		return allowance === 0
			? "You may not keep recordings here."
			: "You keep all the recordings you may keep. Stop keeping one to keep another.";
	if (kept >= cap)
		return cap === 0
			? "This server can't keep recordings yet: the bot's operators decide how many."
			: "This server keeps all the recordings it may keep.";
	return null;
}

/** A recording, by who spoke where and when: the accessible name of its row. */
export function recordingLabel(
	recording: Pick<
		RemovedRecording,
		"speaker_name" | "speaker_id" | "channel_name" | "channel_id"
	> & { started_at_ms?: number | null },
): string {
	const speaker = recording.speaker_name ?? `User ${recording.speaker_id}`;
	const channel = recording.channel_name ?? `channel ${recording.channel_id}`;
	const day = recording.started_at_ms
		? `, ${when(recording.started_at_ms)}`
		: "";
	return `${speaker} in ${channel}${day}`;
}

/** Who keeps a kept recording, from the viewer's side. */
export function keepers(recording: Pick<KeptRecording, "mine" | "keepers">) {
	const others = recording.keepers - (recording.mine ? 1 : 0);
	if (!recording.mine)
		return `Kept by ${others} member${others === 1 ? "" : "s"}`;
	if (others === 0) return "Kept by you";
	return `Kept by you and ${others} other${others === 1 ? "" : "s"}`;
}

/** Why a recording is gone, and since when. */
export function removal(
	recording: Pick<RemovedRecording, "reason" | "removed_at_ms">,
): string {
	const by = recording.reason === "retention" ? "retention" : "a manager";
	return `Removed by ${by} on ${when(recording.removed_at_ms)}`;
}
