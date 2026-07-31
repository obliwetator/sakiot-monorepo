import {
	type Dirs,
	type IndividualFile,
	type MonthNumber,
	PATH_PREFIX_FOR_LOGGED_USERS,
} from "../../../Constants";

export interface AudioTreeRouteParams {
	channel_id?: string;
	file_name?: string;
	month?: string;
	session_id?: string;
	year?: string;
}

export interface AudioTreeRouteState {
	expandedItems: string[];
	selectedItemId: string | null;
}

interface RecordingTreeEntry {
	expandedItems: string[];
	file: IndividualFile;
	itemId: string;
	month: number;
	year: number;
}

export function recordingFileStem(fileName: string): string {
	return fileName.toLowerCase().endsWith(".ogg")
		? fileName.slice(0, -4)
		: fileName;
}

export function recordingFileDay(fileName: string): number {
	return new Date(Number(fileName.slice(0, 13))).getDate();
}

export function recordingTreeItemId(
	file: IndividualFile,
	year: number,
	month: number,
): string {
	const sessionId = file.recording_session_id?.trim();
	if (sessionId) return `recording-session:${encodeURIComponent(sessionId)}`;

	return [
		"recording-file",
		encodeURIComponent(file.channel_id ?? ""),
		year,
		month,
		encodeURIComponent(recordingFileStem(file.file)),
	].join(":");
}

export function recordingTreeItemPath(
	file: IndividualFile,
	year: number,
	month: number,
	guildId: string,
): string | null {
	const root = `${PATH_PREFIX_FOR_LOGGED_USERS}/${encodeURIComponent(guildId)}/audio`;
	const sessionId = file.recording_session_id?.trim();
	if (sessionId) return `${root}/session/${encodeURIComponent(sessionId)}`;
	if (!file.channel_id) return null;

	return `${root}/${encodeURIComponent(file.channel_id)}/${year}/${month}/${encodeURIComponent(recordingFileStem(file.file))}`;
}

function recordingEntries(data: Dirs[]): RecordingTreeEntry[] {
	const entries: RecordingTreeEntry[] = [];
	for (const directory of data) {
		for (const [monthKey, files] of Object.entries(directory.months)) {
			const month = Number(monthKey);
			for (const file of files ?? []) {
				const day = recordingFileDay(file.file);
				entries.push({
					expandedItems: [
						`${directory.year}`,
						`${directory.year}-${month}`,
						`${directory.year}-${month}-${day}`,
					],
					file,
					itemId: recordingTreeItemId(file, directory.year, month),
					month,
					year: directory.year,
				});
			}
		}
	}
	return entries;
}

function decoded(value: string): string {
	try {
		return decodeURIComponent(value);
	} catch {
		return value;
	}
}

function physicalUrlExpansion(params: AudioTreeRouteParams): string[] | null {
	const year = Number(params.year);
	const month = Number(params.month);
	const fileTimestamp = Number(params.file_name?.slice(0, 13));
	if (
		!params.file_name ||
		!Number.isFinite(year) ||
		!Number.isFinite(month) ||
		!Number.isFinite(fileTimestamp)
	) {
		return null;
	}

	const day = new Date(fileTimestamp).getDate();
	return [`${year}`, `${year}-${month}`, `${year}-${month}-${day}`];
}

// Expansion path to the topmost (newest) day actually present in the tree.
// Mirrors the render order: years desc, months desc, days desc.
export function topmostTreeExpansion(data: Dirs[]): string[] {
	const year = data[0];
	if (!year) return [];

	const topMonth = Object.keys(year.months)
		.map(Number)
		.sort((a, b) => b - a)[0];
	if (topMonth === undefined) return [`${year.year}`];

	const files = year.months[topMonth as MonthNumber] ?? [];
	let topDay: number | undefined;
	for (const file of files) {
		const day = recordingFileDay(file.file);
		if (topDay === undefined || day > topDay) topDay = day;
	}

	const ids = [`${year.year}`, `${year.year}-${topMonth}`];
	if (topDay !== undefined) ids.push(`${year.year}-${topMonth}-${topDay}`);
	return ids;
}

export function audioTreeRouteState(
	data: Dirs[],
	params: AudioTreeRouteParams,
): AudioTreeRouteState {
	const entries = recordingEntries(data);
	const sessionId = params.session_id?.trim();
	let selected: RecordingTreeEntry | undefined;

	if (sessionId) {
		selected = entries.find(
			(entry) => entry.file.recording_session_id?.trim() === sessionId,
		);
	} else if (params.file_name) {
		const year = Number(params.year);
		const month = Number(params.month);
		const routeStems = new Set([
			recordingFileStem(params.file_name),
			recordingFileStem(decoded(params.file_name)),
		]);
		selected = entries.find(
			(entry) =>
				entry.year === year &&
				entry.month === month &&
				entry.file.channel_id === params.channel_id &&
				routeStems.has(recordingFileStem(entry.file.file)),
		);
	}

	return {
		expandedItems:
			selected?.expandedItems ??
			physicalUrlExpansion(params) ??
			topmostTreeExpansion(data),
		selectedItemId: selected?.itemId ?? null,
	};
}

export function recordingTreeRoutes(
	data: Dirs[],
	guildId: string,
): ReadonlyMap<string, string> {
	const routes = new Map<string, string>();
	for (const entry of recordingEntries(data)) {
		const path = recordingTreeItemPath(
			entry.file,
			entry.year,
			entry.month,
			guildId,
		);
		if (path) routes.set(entry.itemId, path);
	}
	return routes;
}
