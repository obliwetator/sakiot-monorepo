import { test as base, expect, type Route } from "@playwright/test";

interface ConsoleAudit {
	/** Allow only a deliberately exercised browser diagnostic in this test. */
	allow: (pattern: RegExp) => void;
}

export { expect };

/** Every guild-wide capability, as the guild list reports an owner's. */
export const OWNER_CAPABILITIES = [
	"manage_settings",
	"manage_permissions",
	"preview_roles",
	"delete_recordings",
	"moderate_clips",
];

/**
 * Redux Toolkit's development checks warn when they take more than 32 ms. On a
 * loaded CI runner that measures the machine, not the app, so no test fails on
 * it; developers still see it in the browser.
 */
const MACHINE_SPEED_WARNINGS = [
	/(?:Immutable|Serializable)StateInvariantMiddleware took \d+ms/,
];

export const test = base.extend<{
	consoleAudit: ConsoleAudit;
	realtimeSocket: undefined;
}>({
	// Every logged-in page opens the realtime socket. Unless a test plays the
	// server side itself (its own `page.routeWebSocket` takes precedence),
	// the socket opens but the server never sends `ready`: realtime stays
	// `connecting` and pages keep polling.
	realtimeSocket: [
		async ({ page }, use) => {
			await page.routeWebSocket(/\/api\/realtime$/, () => {
				// Silent server.
			});
			await use(undefined);
		},
		{ auto: true },
	],
	consoleAudit: [
		async ({ page }, use) => {
			const diagnostics: string[] = [];
			const allowed: RegExp[] = [...MACHINE_SPEED_WARNINGS];
			page.on("console", (message) => {
				if (message.type() === "warning" || message.type() === "error") {
					const url = message.location().url;
					diagnostics.push(
						`[${message.type()}] ${message.text()}${url ? ` @ ${url}` : ""}`,
					);
				}
			});
			page.on("pageerror", (error) => {
				diagnostics.push(`[pageerror] ${error.message}`);
			});
			await use({ allow: (pattern) => allowed.push(pattern) });
			expect(
				diagnostics.filter(
					(message) => !allowed.some((pattern) => pattern.test(message)),
				),
				"Unexpected browser console warning/error",
			).toEqual([]);
		},
		{ auto: true },
	],
});

const RECORDING_OPT_OUT_PATH =
	/\/api\/users\/current\/guilds\/[^/]+\/recording-opt-out$/;
const RECORDING_PRIVACY_PATH =
	/\/api\/users\/current\/guilds\/[^/]+\/recording-privacy$/;
const VOICE_PRESENCE_PATH = /\/api\/current\/[^/]+\/voice-presence$/;

/**
 * Answers the endpoints every page queries on its own, so catch-all API mocks
 * can try it before their own routes (it returns false for anything else):
 *
 * - the recording opt-out and listening consent the account menu shows for
 *   the selected server, as a member who is recorded, shares their
 *   recordings, and whose changes are accepted;
 * - voice presence for the recordings sidebar, with nobody in voice.
 */
export async function fulfillSharedRoutes(
	route: Route,
	headers: Record<string, string>,
): Promise<boolean> {
	const request = route.request();
	const path = new URL(request.url()).pathname;
	const json = { ...headers, "Content-Type": "application/json" };
	if (VOICE_PRESENCE_PATH.test(path)) {
		await route.fulfill({
			status: 200,
			headers: json,
			body: JSON.stringify({ available: true, channels: [] }),
		});
		return true;
	}
	const initial = RECORDING_OPT_OUT_PATH.test(path)
		? { opted_out: false }
		: RECORDING_PRIVACY_PATH.test(path)
			? { private: false }
			: null;
	if (!initial) {
		return false;
	}
	await route.fulfill({
		status: 200,
		headers: json,
		body:
			request.method() === "PUT"
				? (request.postData() ?? "{}")
				: JSON.stringify(initial),
	});
	return true;
}

/**
 * Stalls a mock's reads as a slow refetch would: after `hold()`, `passed()`
 * waits until `release()`.
 */
export function heldReads() {
	let gate = Promise.resolve();
	let open = () => {};
	return {
		hold() {
			gate = new Promise((resolve) => {
				open = resolve;
			});
		},
		release: () => open(),
		passed: () => gate,
	};
}
