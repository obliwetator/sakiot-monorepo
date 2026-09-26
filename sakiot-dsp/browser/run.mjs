import { chromium } from "playwright";
import { startHarnessServer } from "./server.mjs";

const server = await startHarnessServer();
const browserLibraryPath = process.env.SAKIOT_BROWSER_LIBRARY_PATH;
const browser = await chromium.launch({
	headless: true,
	args: ["--autoplay-policy=no-user-gesture-required"],
	...(browserLibraryPath
		? { env: { ...process.env, LD_LIBRARY_PATH: browserLibraryPath } }
		: {}),
});
try {
	const page = await browser.newPage();
	page.on("console", (message) => console.log(`[browser:${message.type()}] ${message.text()}`));
	page.on("pageerror", (error) =>
		console.error(
			`[browser:error] name=${error.name} message=${error.message} stack=${error.stack}`,
		),
	);
	page.on("requestfailed", (request) =>
		console.error(`[request:failed] ${request.url()} ${request.failure()?.errorText}`),
	);
	page.on("response", (response) => {
		if (response.status() >= 400) {
			console.error(`[response:${response.status()}] ${response.url()}`);
		}
	});
	await page.goto(server.url, { waitUntil: "networkidle" });
	await page.waitForFunction(() => typeof window.runParity === "function", null, {
		timeout: 10_000,
	});
	const result = await page.evaluate(() => window.runParity());
	for (const measurement of result.measurements) {
		console.log(
			`${measurement.fixture.padStart(7)} ${measurement.effect.padStart(8)}: ` +
				`relative=${measurement.relativeResidualDb.toFixed(2)} dB, ` +
				`max_abs=${measurement.maxAbsolute.toExponential(3)}` +
				(measurement.maxAbsolute > 1e-4
					? `, peak=${JSON.stringify(measurement.maxLocation)}`
					: ""),
		);
		if (measurement.toneEvents) {
			console.log(`  tone events: ${JSON.stringify(measurement.toneEvents)}`);
			console.log(`  wasm events: ${JSON.stringify(measurement.wasmEvents)}`);
		}
		if (measurement.effect === "chorus") {
			console.log(
				`  channels: ${measurement.channelRelativeResidualDb
					.map((value) => value.toFixed(2))
					.join(" dB, ")} dB`,
			);
		}
	}
	console.log(`worklet: ${JSON.stringify(result.worklet)}`);
} finally {
	await browser.close();
	await server.close();
}
