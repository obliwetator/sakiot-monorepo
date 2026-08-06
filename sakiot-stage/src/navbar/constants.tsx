import AdminPanelSettingsIcon from "@mui/icons-material/AdminPanelSettings";
import AudiotrackIcon from "@mui/icons-material/Audiotrack";
import BookmarkIcon from "@mui/icons-material/Bookmark";
import GroupsIcon from "@mui/icons-material/Groups";
import MovieIcon from "@mui/icons-material/Movie";
import SettingsVoiceIcon from "@mui/icons-material/SettingsVoice";
import type * as React from "react";

export type PageName =
	| "Audio"
	| "Clips"
	| "Stamps"
	| "Admin"
	| "Voice Settings"
	| "Members";

export const pages: PageName[] = ["Audio", "Clips", "Stamps"];
export const settings = ["Profile", "Account", "Logout"];

export const pageIcons: Record<PageName, React.ReactElement> = {
	Audio: <AudiotrackIcon />,
	Clips: <MovieIcon />,
	Stamps: <BookmarkIcon />,
	Admin: <AdminPanelSettingsIcon />,
	"Voice Settings": <SettingsVoiceIcon />,
	Members: <GroupsIcon />,
};
