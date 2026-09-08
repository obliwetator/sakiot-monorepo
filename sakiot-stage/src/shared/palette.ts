/**
 * Colour values for APIs that take strings instead of class names: canvas
 * contexts, SVG presentation attributes, and wavesurfer options. Tailwind
 * classes cannot reach those, so the values are declared once here and
 * referenced everywhere else.
 *
 * `src/index.css` (`@theme`) is the equivalent single source for CSS and
 * Tailwind utilities. Together those two files are the only places in the
 * frontend allowed to hold a colour literal; `palette.test.ts` enforces it.
 */
export const palette = {
	red500: "#ef4444",
	rose400: "#fb7185",
	purple500: "#a855f7",
	purple400: "#c084fc",
	fuchsia500: "#d946ef",
	yellow500: "#eab308",
	green500: "#22c55e",
	teal500: "#14b8a6",
	teal400: "#2dd4bf",
	orange500: "#f97316",
	orange400: "#fb923c",
	sky500: "#0ea5e9",
	sky300: "#7dd3fc",
	cyan500: "#06b6d4",
	cyan200: "#a5f3fc",
	slate300: "#cbd5e1",
	slate400: "#94a3b8",
	slate500: "#64748b",
	slate900: "#0f172a",
	white: "#ffffff",
	/** wavesurfer waveform fill for the manual generate widget. */
	magenta: "#ff00ff",
	magentaDark: "#cc00cc",
	/** wavesurfer timeline label text. */
	waveformLabel: "#6a6a6a",
} as const;

/** `rgba()` string for a palette hex at the given alpha. */
export function alpha(hex: string, value: number): string {
	const channels = Number.parseInt(hex.slice(1), 16);
	return `rgba(${(channels >> 16) & 255}, ${(channels >> 8) & 255}, ${channels & 255}, ${value})`;
}
