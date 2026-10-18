import {
	ShieldCheck as AdminPanelSettingsIcon,
	AudioLines as AudiotrackIcon,
	Bookmark as BookmarkIcon,
	Scissors as ContentCutIcon,
	Users as GroupsIcon,
	Film as MovieIcon,
	Mic as SettingsVoiceIcon,
} from "lucide-react";
import type * as React from "react";
import type { UserGuilds } from "../Constants";
import { type Capability, holds, isFormerMember } from "../shared/permissions";

export type PageName =
	| "Audio"
	| "Clips"
	| "Clip Editor"
	| "Stamps"
	| "Admin"
	| "Voice Settings"
	| "Members";

export const pages: PageName[] = ["Audio", "Clips", "Clip Editor", "Stamps"];

/** The admin pages, each shown to holders of the capability it needs. */
export const adminPages: { page: PageName; capability: Capability }[] = [
	{ page: "Admin", capability: "manage_settings" },
	{ page: "Voice Settings", capability: "manage_settings" },
	{ page: "Members", capability: "preview_roles" },
];

/** What a former member can open: their own recordings, clips and stamps. */
const formerMemberPages: PageName[] = ["Audio", "Clips", "Stamps"];

/** The pages the navigation offers in `guild`. */
export function guildPages(guild: UserGuilds | null): PageName[] {
	if (isFormerMember(guild)) return formerMemberPages;
	return [
		...pages,
		...adminPages
			.filter(({ capability }) => holds(guild, capability))
			.map(({ page }) => page),
	];
}

export function activePage(pathname: string): PageName | null {
	if (pathname.startsWith("/stamps")) return "Stamps";
	const parts = pathname.split("/").filter(Boolean);
	if (parts[0] !== "dashboard" || !parts[1]) return null;
	switch (parts[2]) {
		case "audio":
			return "Audio";
		case "clips":
			return parts[3] === "editor" ? "Clip Editor" : "Clips";
		case "members":
			return "Members";
		case "admin":
			return parts[3] === "voice-settings" ? "Voice Settings" : "Admin";
		default:
			return null;
	}
}

/**
 * Path under a guild for each page. Stamps is absent: it has its own
 * top-level route and is the one page reachable without a guild.
 */
export const guildPagePaths: Record<Exclude<PageName, "Stamps">, string> = {
	Audio: "audio",
	Clips: "clips",
	"Clip Editor": "clips/editor",
	Admin: "admin/cooldowns",
	"Voice Settings": "admin/voice-settings",
	Members: "members",
};

export const pageIcons: Record<PageName, React.ReactElement> = {
	Audio: <AudiotrackIcon />,
	Clips: <MovieIcon />,
	"Clip Editor": <ContentCutIcon />,
	Stamps: <BookmarkIcon />,
	Admin: <AdminPanelSettingsIcon />,
	"Voice Settings": <SettingsVoiceIcon />,
	Members: <GroupsIcon />,
};
