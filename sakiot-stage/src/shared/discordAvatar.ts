const DISCORD_CDN = "https://cdn.discordapp.com";
/** Discord has six default avatar colours, chosen from the user snowflake. */
const DEFAULT_AVATAR_COUNT = 6n;
/** Rendered at 40px in the navbar; 64 covers 2x displays. */
const AVATAR_SIZE = 64;

export interface DiscordAvatarUser {
	user_id: string;
	avatar: string;
}

/**
 * Avatar URL for a Discord account, or `null` when the payload cannot produce
 * one (no user, or an id that is not a snowflake — fixtures and dev logins use
 * non-numeric ids). Callers fall back to the username's initial.
 *
 * Discord returns an empty `avatar` hash for accounts that never uploaded one;
 * those get the deterministic default avatar for their snowflake rather than a
 * shared placeholder image.
 */
export function discordAvatarUrl(
	user: DiscordAvatarUser | null | undefined,
): string | null {
	if (!user) return null;

	const hash = user.avatar.trim();
	if (hash) {
		// Animated avatars are served as GIF; everything else as PNG.
		const extension = hash.startsWith("a_") ? "gif" : "png";
		return `${DISCORD_CDN}/avatars/${user.user_id}/${hash}.${extension}?size=${AVATAR_SIZE}`;
	}

	const index = defaultAvatarIndex(user.user_id);
	if (index === null) return null;
	return `${DISCORD_CDN}/embed/avatars/${index}.png`;
}

function defaultAvatarIndex(userId: string): number | null {
	if (!/^[0-9]+$/.test(userId)) return null;
	return Number((BigInt(userId) >> 22n) % DEFAULT_AVATAR_COUNT);
}
