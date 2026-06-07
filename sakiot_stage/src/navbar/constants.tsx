import AdminPanelSettingsIcon from "@mui/icons-material/AdminPanelSettings";
import AudiotrackIcon from "@mui/icons-material/Audiotrack";
import BarChartIcon from "@mui/icons-material/BarChart";
import BookmarkIcon from "@mui/icons-material/Bookmark";
import MovieIcon from "@mui/icons-material/Movie";
import type * as React from "react";

export type PageName = "Audio" | "Clips" | "Metrics" | "Stamps" | "Admin";

export const pages: PageName[] = ["Audio", "Clips", "Metrics", "Stamps"];
export const settings = ["Profile", "Account", "Metrics", "Logout"];

export const pageIcons: Record<PageName, React.ReactElement> = {
	Audio: <AudiotrackIcon />,
	Clips: <MovieIcon />,
	Metrics: <BarChartIcon />,
	Stamps: <BookmarkIcon />,
	Admin: <AdminPanelSettingsIcon />,
};
