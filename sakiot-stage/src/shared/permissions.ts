import type { components } from "../api/openapi";
import type { UserGuilds } from "../Constants";

export type Capability = components["schemas"]["Capability"];

/** Whether the viewer holds `capability` across the guild. */
export function holds(
	guild: UserGuilds | null | undefined,
	capability: Capability,
): boolean {
	return guild?.capabilities.includes(capability) ?? false;
}

/** Renaming or deleting a clip: its creator, or a clip moderator. */
export function canDeleteClip(
	guild: UserGuilds | null,
	currentUserId: string | null | undefined,
	clipOwnerId: string | null | undefined,
): boolean {
	if (holds(guild, "moderate_clips")) return true;
	return Boolean(currentUserId && clipOwnerId && currentUserId === clipOwnerId);
}
