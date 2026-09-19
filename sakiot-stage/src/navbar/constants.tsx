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

export type PageName =
	| "Audio"
	| "Clips"
	| "Clip Editor"
	| "Stamps"
	| "Admin"
	| "Voice Settings"
	| "Members";

export const pages: PageName[] = ["Audio", "Clips", "Clip Editor", "Stamps"];

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
export const settings = ["Profile", "Account", "Logout"];

export const pageIcons: Record<PageName, React.ReactElement> = {
	Audio: <AudiotrackIcon />,
	Clips: <MovieIcon />,
	"Clip Editor": <ContentCutIcon />,
	Stamps: <BookmarkIcon />,
	Admin: <AdminPanelSettingsIcon />,
	"Voice Settings": <SettingsVoiceIcon />,
	Members: <GroupsIcon />,
};
