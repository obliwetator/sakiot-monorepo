import { defineConfig } from "@playwright/test";
import config from "./playwright.config";

// Exercise real chunk loading with the same local/mock APIs as the dev suite.
export default defineConfig({
	...config,
	outputDir: "test-results/production",
	webServer: {
		command:
			"VITE_API_URL=http://127.0.0.1:4174/api/ bun run build:bundle && bunx vite preview --host 127.0.0.1 --port 4173 --strictPort",
		url: "http://127.0.0.1:4173",
		reuseExistingServer: false,
		timeout: 120_000,
	},
});
