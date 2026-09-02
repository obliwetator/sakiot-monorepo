import {
	type CSSProperties,
	createElement,
	type ElementType,
	type ReactNode,
	useEffect,
	useMemo,
	useState,
} from "react";

export interface Theme {
	breakpoints: { up: (name: string) => string };
	zIndex: { modal: number; snackbar: number; tooltip: number };
}

export type SxProps<_T = Theme> =
	| Record<string, unknown>
	| false
	| null
	| undefined;

export const spacingKeys: Record<string, string> = {
	p: "padding",
	px: "paddingInline",
	py: "paddingBlock",
	pt: "paddingTop",
	pr: "paddingRight",
	pb: "paddingBottom",
	pl: "paddingLeft",
	m: "margin",
	mx: "marginInline",
	my: "marginBlock",
	mt: "marginTop",
	mr: "marginRight",
	mb: "marginBottom",
	ml: "marginLeft",
	gap: "gap",
};

export const cssAliases: Record<string, string> = {
	bgcolor: "backgroundColor",
};

export function spacingValue(value: unknown): string | number | undefined {
	if (typeof value !== "number") return value as string | undefined;
	return value * 8;
}

export function themeValue(value: unknown): unknown {
	if (typeof value !== "string") return value;
	const values: Record<string, string> = {
		"background.default": "#18181b",
		"background.paper": "#202023",
		"text.primary": "#f8fafc",
		"text.secondary": "#94a3b8",
		"text.disabled": "#64748b",
		"primary.main": "#90caf9",
		"primary.light": "#67e8f9",
		"primary.dark": "#0891b2",
		"secondary.main": "#a78bfa",
		"secondary.light": "#c4b5fd",
		"secondary.dark": "#7c3aed",
		"error.main": "#f87171",
		"error.light": "#fca5a5",
		"error.dark": "#dc2626",
		"error.contrastText": "#180b0b",
		"warning.main": "#fbbf24",
		"warning.light": "#fcd34d",
		"warning.dark": "#d97706",
		"success.main": "#34d399",
		"success.light": "#6ee7b7",
		"success.dark": "#059669",
		"info.main": "#38bdf8",
		"info.light": "#7dd3fc",
		"info.dark": "#0284c7",
		divider: "#3f3f46",
		white: "#fff",
	};
	return values[value] ?? value;
}

export function resolveResponsiveValue(value: unknown): unknown {
	if (typeof value !== "object" || value === null || Array.isArray(value)) {
		return value;
	}
	const responsive = value as Record<string, unknown>;
	const width = typeof window !== "undefined" ? window.innerWidth : 0;
	if (width >= 900) return responsive.md ?? responsive.sm ?? responsive.xs;
	if (width >= 600) return responsive.sm ?? responsive.xs ?? responsive.md;
	return responsive.xs ?? responsive.sm ?? responsive.md;
}

export function sxToStyle(sx: SxProps): CSSProperties {
	if (!sx || typeof sx !== "object" || Array.isArray(sx)) return {};
	const style: Record<string, unknown> = {};
	for (const [key, rawValue] of Object.entries(sx)) {
		if (key.startsWith("&") || key.startsWith("@")) continue;
		const value = resolveResponsiveValue(rawValue);
		if (value === undefined) continue;
		const cssKey = cssAliases[key] ?? spacingKeys[key] ?? key;
		const borderShorthand =
			key === "border" ||
			key === "borderTop" ||
			key === "borderRight" ||
			key === "borderBottom" ||
			key === "borderLeft";
		const cssValue =
			borderShorthand && typeof value === "number" ? `${value}px solid` : value;
		style[cssKey] = spacingKeys[key]
			? spacingValue(value)
			: themeValue(cssValue);
	}
	if (style.borderColor !== undefined) {
		const borderColor = style.borderColor;
		for (const side of ["Top", "Right", "Bottom", "Left"]) {
			style[`border${side}Color`] = borderColor;
		}
		delete style.borderColor;
	}
	return style as CSSProperties;
}

export function mergedStyle(sx: SxProps, style?: CSSProperties): CSSProperties {
	return { ...sxToStyle(sx), ...style };
}

export function omitCompatProps<T extends Record<string, unknown>>(
	props: T,
): Partial<T> {
	const result = { ...props };
	const forbiddenKeys = new Set([
		"sx",
		"variant",
		"color",
		"size",
		"fullWidth",
		"disableGutters",
		"disablePadding",
		"disableRipple",
		"display",
		"alignItems",
		"justifyContent",
		"flexGrow",
		"flexShrink",
		"flex",
		"flexDirection",
		"flexWrap",
		"minHeight",
		"minWidth",
		"width",
		"height",
		"maxWidth",
		"maxHeight",
		"overflow",
		"overflowX",
		"overflowY",
		"position",
		"top",
		"right",
		"bottom",
		"left",
		"zIndex",
		"border",
		"borderColor",
		"borderRadius",
		"backgroundColor",
		"bgcolor",
		"boxShadow",
		"opacity",
		"pointerEvents",
		"userSelect",
		"touchAction",
		"clipPath",
		"fontVariantNumeric",
		"textTransform",
		"whiteSpace",
		"transform",
		"transition",
		"gridTemplateColumns",
		"gridColumn",
		"labelId",
		"gutterBottom",
		"noWrap",
		"textAlign",
		"fontSize",
		"fontWeight",
		"margin",
		"dense",
		"primary",
		"secondary",
		"primaryTypographyProps",
		"secondaryTypographyProps",
		"label",
		"action",
		"severity",
		"icon",
		"dividers",
		"maxWidth",
		"anchor",
		"anchorEl",
		"anchorOrigin",
		"transformOrigin",
		"keepMounted",
		"ModalProps",
		"transitionDuration",
		"unmountOnExit",
		"TransitionComponent",
		"slots",
		"slotProps",
		"inputProps",
		"helperText",
		"minRows",
		"maxRows",
		"valueLabelDisplay",
		"valueLabelFormat",
		"getAriaLabel",
		"getAriaValueText",
		"disableSwap",
		"onChangeCommitted",
		"onChangeEnd",
		"orientation",
		"marks",
		"expanded",
		"onExpandedItemsChange",
		"expandedItems",
		"selectedItems",
		"onSelectedItemsChange",
		"onItemClick",
	]);
	for (const key of forbiddenKeys) {
		delete (result as Record<string, unknown>)[key];
	}
	return result;
}

export function resolveTag(
	component: ElementType | undefined,
	fallback: ElementType,
): ElementType {
	return component ?? fallback;
}

export function useTheme(): Theme {
	return useMemo<Theme>(
		() => ({
			breakpoints: {
				up: (name: string) => `(min-width: ${name === "md" ? 900 : 0}px)`,
			},
			zIndex: { modal: 50, snackbar: 60, tooltip: 70 },
		}),
		[],
	);
}

export function useMediaQuery(query: string): boolean {
	const [matches, setMatches] = useState(
		() => typeof window !== "undefined" && window.matchMedia(query).matches,
	);
	useEffect(() => {
		const media = window.matchMedia(query);
		const update = () => setMatches(media.matches);
		update();
		media.addEventListener("change", update);
		return () => media.removeEventListener("change", update);
	}, [query]);
	return matches;
}

export function createTheme(theme: Record<string, unknown>) {
	return theme;
}

export function CssBaseline() {
	return null;
}

export function GlobalStyles(_props: Record<string, unknown>) {
	return null;
}

export function StyledEngineProvider({
	children,
}: {
	children: ReactNode;
	enableCssLayer?: boolean;
}) {
	return children;
}

export function ThemeProvider({
	children,
}: {
	children: ReactNode;
	theme?: unknown;
}) {
	return children;
}

export function keyframes(
	frames: TemplateStringsArray | Record<string, unknown>,
) {
	return Array.isArray(frames) ? frames.join("") : JSON.stringify(frames);
}

export function styled(Component: ElementType) {
	return (
		styles:
			| Record<string, unknown>
			| ((props: Record<string, unknown>) => Record<string, unknown>)
			| TemplateStringsArray,
	) =>
		(props: Record<string, unknown>) => {
			const resolved = Array.isArray(styles)
				? {}
				: typeof styles === "function"
					? styles(props)
					: styles;
			return createElement(Component, {
				...props,
				sx: {
					...resolved,
					...(props.sx as Record<string, unknown> | undefined),
				},
			});
		};
}
