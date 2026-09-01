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
