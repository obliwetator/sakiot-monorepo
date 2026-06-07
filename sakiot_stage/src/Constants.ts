import type { components } from "./api/openapi";

export const BASE_URL = "https://dev.patrykstyla.com/";

export const PATH_PREFIX_FOR_LOGGED_USERS = "/dashboard";
export function valuetext(value: number) {
	return `${value}°C`;
}

type ApiSchema = components["schemas"];

export type Channels = ApiSchema["Channels"];

export type AudioParams2 = {
	dashboard?: string;
	audio?: string;
	guild_id?: string;
	channel_id?: string;
	file_name?: string;
	month?: number;
	year?: string;
};
export type AudioParams =
	| "guild_id"
	| "channel_id"
	| "file_name"
	| "month"
	| "year";

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

export type months = number;

export type UserGuilds = ApiSchema["GuildDataForFrontEnd"];
