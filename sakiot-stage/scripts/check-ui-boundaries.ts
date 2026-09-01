import { readdir, readFile } from "node:fs/promises";
import { relative, resolve } from "node:path";

const projectRoot = resolve(import.meta.dir, "..");
const sourceRoot = resolve(projectRoot, "src");
const packagePath = resolve(projectRoot, "package.json");

const forbiddenSourcePatterns: Array<[RegExp, string]> = [
	[/@mui\//, "MUI import"],
	[/@emotion\//, "Emotion import"],
	[/@react-spring\//, "React Spring import"],
	[/(?:^|[^a-z])react-spring(?:[^a-z]|$)/, "React Spring reference"],
	[/Material Icons|material-icons/, "Material Icons request"],
	[/\.Mui[A-Z]|\bMui[A-Z]/, "MUI class or component reference"],
];

async function sourceFiles(directory: string): Promise<string[]> {
	const entries = await readdir(directory, { withFileTypes: true });
	const nested = await Promise.all(
		entries.map(async (entry) => {
			const path = resolve(directory, entry.name);
			if (entry.isDirectory()) return sourceFiles(path);
			return /\.(?:js|jsx|ts|tsx)$/.test(entry.name) ? [path] : [];
		}),
	);
	return nested.flat();
}

const violations: string[] = [];
for (const path of await sourceFiles(sourceRoot)) {
	const repoPath = relative(projectRoot, path).replaceAll("\\", "/");
	const source = await readFile(path, "utf8");
	for (const [pattern, description] of forbiddenSourcePatterns) {
		if (pattern.test(source)) violations.push(`${repoPath}: ${description}`);
	}
	if (
		/['"]react-aria-components(?:\/|['"])/.test(source) &&
		!repoPath.startsWith("src/shared/ui/")
	) {
		violations.push(`${repoPath}: React Aria must be wrapped by shared/ui`);
	}
}

const packageJson = JSON.parse(await readFile(packagePath, "utf8")) as {
	dependencies?: Record<string, string>;
	devDependencies?: Record<string, string>;
};
const dependencies = {
	...packageJson.dependencies,
	...packageJson.devDependencies,
};
for (const dependency of Object.keys(dependencies)) {
	if (
		dependency.startsWith("@mui/") ||
		dependency.startsWith("@emotion/") ||
		dependency === "react-spring" ||
		dependency.startsWith("@react-spring/")
	) {
		violations.push(`package.json: forbidden dependency ${dependency}`);
	}
}

for (const path of [resolve(projectRoot, "index.html"), packagePath]) {
	const source = await readFile(path, "utf8");
	for (const [pattern, description] of forbiddenSourcePatterns) {
		if (pattern.test(source)) {
			violations.push(
				`${relative(projectRoot, path).replaceAll("\\", "/")}: ${description}`,
			);
		}
	}
}

if (violations.length > 0) {
	console.error(`UI boundary check failed:\n${violations.join("\n")}`);
	process.exit(1);
}

console.log("UI boundary check passed.");
