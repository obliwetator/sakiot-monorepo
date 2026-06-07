// import react from '@vitejs/plugin-react';
import react from "@vitejs/plugin-react";
import { visualizer } from "rollup-plugin-visualizer";
import { defineConfig, type Plugin } from "vite";

const bundleBuiltAt = new Date().toISOString();
const bundleVersion = `${Date.now()}`;

function bundleVersionPlugin(): Plugin {
	return {
		name: "bundle-version",
		apply: "build",
		generateBundle() {
			this.emitFile({
				type: "asset",
				fileName: "version.json",
				source: `${JSON.stringify(
					{ version: bundleVersion, builtAt: bundleBuiltAt },
					null,
					2,
				)}\n`,
			});
		},
	};
}

// https://vitejs.dev/config/
export default defineConfig({
	define: {
		__BUNDLE_VERSION__: JSON.stringify(bundleVersion),
	},
	plugins: [
		bundleVersionPlugin(),
		react({
			babel: {
				plugins: [["babel-plugin-react-compiler", {}]],
			},
		}),
		visualizer({
			filename: "dist/stats.html",
			gzipSize: true,
			brotliSize: true,
		}),
	],
	server: {
		port: 8081,
		allowedHosts: [
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
});
