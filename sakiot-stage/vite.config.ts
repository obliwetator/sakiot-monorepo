import babel from "@rolldown/plugin-babel";
import react, { reactCompilerPreset } from "@vitejs/plugin-react";
import { visualizer } from "rollup-plugin-visualizer";
import { defineConfig, loadEnv, type Plugin } from "vite";

const bundleBuiltAt = new Date().toISOString();
const releaseTag = process.env.SAKIOT_RELEASE_TAG ?? "development";
const commitSha = process.env.SAKIOT_COMMIT_SHA ?? "unknown";
const bundleVersion =
	process.env.SAKIOT_BUNDLE_VERSION ??
	`${releaseTag}-${commitSha}-${Date.now()}`;

function bundleVersionPlugin(): Plugin {
	return {
		name: "bundle-version",
		apply: "build",
		generateBundle() {
			this.emitFile({
				type: "asset",
				fileName: "version.json",
				source: `${JSON.stringify(
					{
						version: bundleVersion,
						releaseTag,
						commitSha,
						builtAt: bundleBuiltAt,
					},
					null,
					2,
				)}\n`,
			});
		},
	};
}

// https://vitejs.dev/config/
export default defineConfig(({ command, mode }) => {
	// The bundle refuses to start without an API origin (src/app/authedFetch.ts);
	// surface a missing VITE_API_URL at build time instead of on first page load.
	if (
		command === "build" &&
		!process.env.VITE_API_URL &&
		!loadEnv(mode, "..", "VITE_").VITE_API_URL
	) {
		throw new Error(
			"VITE_API_URL is not set (env file or environment); refusing to build a bundle without an API origin.",
		);
	}
	return {
		envDir: "..",
		define: {
			__BUNDLE_VERSION__: JSON.stringify(bundleVersion),
		},
		plugins: [
			bundleVersionPlugin(),
			react(),
			// plugin-react v6 transforms JSX with oxc and no longer accepts a
			// `babel` option, so the React Compiler runs as its own preset.
			babel({ presets: [reactCompilerPreset()] }),
			visualizer({
				filename: "dist/stats.html",
				gzipSize: true,
				brotliSize: true,
			}),
		],
		server: {
			port: 8081,
			allowedHosts: [
				"debug.patrykstyla.com",
				"staging.patrykstyla.com",
				"dev.patrykstyla.com",
				"patrykstyla.com",
			],
		},
		build: {
			rolldownOptions: {
				output: {
					codeSplitting: {
						groups: [
							{
								name: "react",
								test: /node_modules\/(react|react-dom|react-router|react-router-dom|scheduler)\//,
							},
							{
								name: "redux",
								test: /node_modules\/(@reduxjs\/toolkit|react-redux|redux|reselect|immer)\//,
							},
							{
								name: "mui",
								test: /node_modules\/(@mui|@emotion)\//,
							},
						],
					},
				},
			},
		},
	};
});
