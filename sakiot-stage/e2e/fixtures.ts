import { test as base, expect, type Route } from "@playwright/test";

interface ConsoleAudit {
	/** Allow only a deliberately exercised browser diagnostic in this test. */
	allow: (pattern: RegExp) => void;
}

export { expect };

export const test = base.extend<{ consoleAudit: ConsoleAudit }>({
	consoleAudit: [
		async ({ page }, use) => {
			const diagnostics: string[] = [];
			const allowed: RegExp[] = [];
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

/**
 * Answers the recording opt-out endpoint the account menu queries for the
 * selected server, as a member who is recorded and whose changes are
 * accepted. Returns false for any other request, so catch-all API mocks can
 * try it before their own routes.
 */
export async function fulfillRecordingOptOut(
	route: Route,
	headers: Record<string, string>,
): Promise<boolean> {
	const request = route.request();
	if (!RECORDING_OPT_OUT_PATH.test(new URL(request.url()).pathname)) {
		return false;
	}
	await route.fulfill({
		status: 200,
		headers: { ...headers, "Content-Type": "application/json" },
		body:
			request.method() === "PUT"
				? (request.postData() ?? "{}")
				: JSON.stringify({ opted_out: false }),
	});
	return true;
}
