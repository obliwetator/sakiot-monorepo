import type { components } from "./api/openapi";

export const PATH_PREFIX_FOR_LOGGED_USERS = "/dashboard";

type ApiSchema = components["schemas"];

export type Channels = ApiSchema["Channels"];

export type AudioParams =
	| "guild_id"
	| "channel_id"
	| "file_name"
	| "month"
	| "year";

export type MonthNumber = number;

export type Dirs = Omit<ApiSchema["Directories"], "months"> & {
	months: Partial<Record<number, IndividualFileArray>>;
};

export type IndividualFileArray = IndividualFile[];
export type IndividualFile = ApiSchema["File"] & {
	channel_id?: string;
};

export function getMonthName(monthNumber: number): string {
	const months = [
		"Unknown",
		"January",
		"February",
		"March",
		"April",
		"May",
		"June",
		"July",
		"August",
		"September",
		"October",
		"November",
		"December",
	];
	return months[monthNumber] || "Unknown";
}

export type UserGuilds = ApiSchema["GuildDataForFrontEnd"];
