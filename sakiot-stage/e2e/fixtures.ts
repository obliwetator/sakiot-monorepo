import { test as base, expect } from "@playwright/test";

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
