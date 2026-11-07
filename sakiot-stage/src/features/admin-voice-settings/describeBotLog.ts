import type { BotLogEntry } from "../../app/apiSlice";

function channel(name: string | null | undefined): string {
	return name ?? "a deleted channel";
}

/** What happened to the bot. */
export function happened(entry: BotLogEntry): string {
	const from = channel(entry.channel_name);
	switch (entry.kind) {
		case "deafened":
			return `Deafened in ${from}`;
		case "undeafened":
			return `Undeafened in ${from}`;
		case "moved":
			return `Moved from ${from} to ${channel(entry.to_channel_name)}`;
		case "disconnected":
			return `Disconnected from ${from}`;
		case "deafen_clear_failed":
			return `Joined ${from} still deafened and could not remove the deafen. Check the bot has the Deafen Members permission.`;
	}
}

/** Who did it, as far as Discord says; null for the bot's failed cleanup. */
export function culprit(entry: BotLogEntry): string | null {
	if (entry.kind === "deafen_clear_failed") return null;
	const name = entry.actor_name ?? `User ${entry.actor_id}`;
	switch (entry.attribution) {
		case "exact":
			return `By ${name}`;
		case "likely":
			return `Probably by ${name}`;
		case "unknown":
			return "Discord's audit log doesn't say who";
		case "no_access":
			return "Unknown: the bot needs the View Audit Log permission";
	}
}
