import { describe, expect, it } from "bun:test";
import type { Dirs } from "../../../Constants";
import {
	audioTreeRouteState,
	recordingTreeItemId,
	recordingTreeRoutes,
} from "./treeNavigation";

const sessionTimestamp = Date.UTC(2026, 6, 14, 12, 0, 0);
const fileTimestamp = Date.UTC(2026, 6, 15, 12, 0, 0);
const sessionFile = {
	channel_id: "20",
	file: `${sessionTimestamp}-session-user.ogg`,
	recording_session_id: "373",
};
const physicalFile = {
	channel_id: "21",
	file: `${fileTimestamp}-user name.ogg`,
};
const data: Dirs[] = [
	{
		year: 2026,
		months: { 7: [sessionFile, physicalFile] },
	},
];

describe("audioTreeRouteState", () => {
	it("expands to and selects a logical session from its URL", () => {
		expect(audioTreeRouteState(data, { session_id: "373" })).toEqual({
			expandedItems: [
				"2026",
				"2026-7",
				`2026-7-${new Date(sessionTimestamp).getDate()}`,
			],
			selectedItemId: "recording-session:373",
		});
	});

	it("expands to and selects a physical file from encoded URL params", () => {
		const fileName = physicalFile.file.slice(0, -4);
		expect(
			audioTreeRouteState(data, {
				channel_id: "21",
				file_name: encodeURIComponent(fileName),
				month: "7",
				year: "2026",
			}),
		).toEqual({
			expandedItems: [
				"2026",
				"2026-7",
				`2026-7-${new Date(fileTimestamp).getDate()}`,
			],
			selectedItemId: recordingTreeItemId(physicalFile, 2026, 7),
		});
	});

	it("selects the logical tree item when its first fragment URL is opened", () => {
		expect(
			audioTreeRouteState(data, {
				channel_id: "20",
				file_name: sessionFile.file.slice(0, -4),
				month: "7",
				year: "2026",
			}),
		).toEqual({
			expandedItems: [
				"2026",
				"2026-7",
				`2026-7-${new Date(sessionTimestamp).getDate()}`,
			],
			selectedItemId: "recording-session:373",
		});
	});

	it("opens the newest day when the URL does not identify a recording", () => {
		expect(audioTreeRouteState(data, {})).toEqual({
			expandedItems: [
				"2026",
				"2026-7",
				`2026-7-${new Date(fileTimestamp).getDate()}`,
			],
			selectedItemId: null,
		});
	});
});

describe("recordingTreeRoutes", () => {
	it("maps selectable tree items to encoded dashboard routes", () => {
		const routes = recordingTreeRoutes(data, "guild/one");
		expect(routes.get("recording-session:373")).toBe(
			"/dashboard/guild%2Fone/audio/session/373",
		);
		expect(routes.get(recordingTreeItemId(physicalFile, 2026, 7))).toBe(
			`/dashboard/guild%2Fone/audio/21/2026/7/${fileTimestamp}-user%20name`,
		);
	});
});
