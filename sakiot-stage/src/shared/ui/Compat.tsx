import {
	type ButtonHTMLAttributes,
	type ChangeEvent,
	type ChangeEventHandler,
	Children,
	type CSSProperties,
	cloneElement,
	createContext,
	type ElementType,
	forwardRef,
	type HTMLAttributes,
	isValidElement,
	type ReactElement,
	type KeyboardEvent as ReactKeyboardEvent,
	type ReactNode,
	type SyntheticEvent,
	useContext,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { cn } from "./cn";

/**
 * Transitional style input for screens that were authored with MUI's `sx`
 * prop. It deliberately lives in shared/ui so feature code never imports a
 * component library or a styling engine. New code should prefer className.
 */
export interface Theme {
	breakpoints: { up: (name: string) => string };
	zIndex: { modal: number; snackbar: number; tooltip: number };
}
export type SxProps<T = Theme> =
	| Record<string, unknown>
	| false
	| null
	| undefined;

const spacingKeys: Record<string, string> = {
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

const cssAliases: Record<string, string> = {
	bgcolor: "backgroundColor",
};

function spacingValue(value: unknown): string | number | undefined {
	if (typeof value !== "number") return value as string | undefined;
	return value * 8;
}

function themeValue(value: unknown): unknown {
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

function resolveResponsiveValue(value: unknown): unknown {
	if (typeof value !== "object" || value === null || Array.isArray(value)) {
		return value;
	}
	const responsive = value as Record<string, unknown>;
	const width = typeof window !== "undefined" ? window.innerWidth : 0;
	if (width >= 900) return responsive.md ?? responsive.sm ?? responsive.xs;
	if (width >= 600) return responsive.sm ?? responsive.xs ?? responsive.md;
	return responsive.xs ?? responsive.sm ?? responsive.md;
}

function sxToStyle(sx: SxProps): CSSProperties {
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
	// CSS warns when a side shorthand and borderColor are updated together.
	// Expand the shared color to side-specific properties so high-frequency drag
	// rerenders stay warning-free.
	if (style.borderColor !== undefined) {
		const borderColor = style.borderColor;
		for (const side of ["Top", "Right", "Bottom", "Left"]) {
			style[`border${side}Color`] = borderColor;
		}
		delete style.borderColor;
	}
	return style as CSSProperties;
}

function mergedStyle(sx: SxProps, style?: CSSProperties): CSSProperties {
	return { ...sxToStyle(sx), ...style };
}

function omitCompatProps(props: Record<string, unknown>) {
	const result = { ...props };
	for (const key of [
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
		"margin",
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
	])
		delete result[key];
	return result;
}

function resolveTag(component: ElementType | undefined, fallback: ElementType) {
	return component ?? fallback;
}

type BoxProps = HTMLAttributes<HTMLDivElement> & {
	sx?: SxProps;
	component?: ElementType;
	[key: string]: any;
};

export function Box(props: BoxProps) {
	const {
		sx,
		style,
		component,
		p,
		px,
		py,
		pt,
		pr,
		pb,
		pl,
		m,
		mx,
		my,
		mt,
		mr,
		mb,
		ml,
		...rest
	} = props;
	const Component = resolveTag(component, "div");
	const shorthand = {
		...(p !== undefined ? { padding: spacingValue(p) } : {}),
		...(px !== undefined ? { paddingInline: spacingValue(px) } : {}),
		...(py !== undefined ? { paddingBlock: spacingValue(py) } : {}),
		...(pt !== undefined ? { paddingTop: spacingValue(pt) } : {}),
		...(pr !== undefined ? { paddingRight: spacingValue(pr) } : {}),
		...(pb !== undefined ? { paddingBottom: spacingValue(pb) } : {}),
		...(pl !== undefined ? { paddingLeft: spacingValue(pl) } : {}),
		...(m !== undefined ? { margin: spacingValue(m) } : {}),
		...(mx !== undefined ? { marginInline: spacingValue(mx) } : {}),
		...(my !== undefined ? { marginBlock: spacingValue(my) } : {}),
		...(mt !== undefined ? { marginTop: spacingValue(mt) } : {}),
		...(mr !== undefined ? { marginRight: spacingValue(mr) } : {}),
		...(mb !== undefined ? { marginBottom: spacingValue(mb) } : {}),
		...(ml !== undefined ? { marginLeft: spacingValue(ml) } : {}),
	};
	return (
		<Component
			{...omitCompatProps(rest)}
			style={{ ...shorthand, ...sxToStyle(sx), ...style }}
		/>
	);
}

export function Stack(props: any) {
	const {
		direction = "column",
		spacing,
		alignItems,
		justifyContent,
		flexWrap,
		useFlexGap: _useFlexGap,
		sx,
		style,
		className,
		children,
		...rest
	} = props;
	const directionValue = resolveResponsiveValue(direction);
	return (
		<div
			{...omitCompatProps(rest)}
			className={cn(
				"flex",
				directionValue === "row" ? "flex-row" : "flex-col",
				className,
			)}
			style={mergedStyle(sx, {
				gap: spacingValue(resolveResponsiveValue(spacing)),
				alignItems: resolveResponsiveValue(alignItems),
				justifyContent: resolveResponsiveValue(justifyContent),
				flexWrap: resolveResponsiveValue(flexWrap),
				...style,
			})}
		>
			{children}
		</div>
	);
}

const typographyTags: Record<string, ElementType> = {
	h1: "h1",
	h2: "h2",
	h3: "h3",
	h4: "h4",
	h5: "h5",
	h6: "h6",
};

export function Typography(props: any) {
	const {
		variant = "body1",
		color,
		fontSize,
		fontWeight,
		gutterBottom,
		noWrap,
		textAlign,
		display,
		sx,
		style,
		className,
		component,
		children,
		...rest
	} = props;
	const Component = resolveTag(component, typographyTags[variant] ?? "p");
	const defaultSize =
		variant === "h5"
			? "1.5rem"
			: variant === "h6"
				? "1.25rem"
				: variant === "subtitle1"
					? "1rem"
					: variant === "caption"
						? "0.75rem"
						: variant === "body2"
							? "0.875rem"
							: undefined;
	return (
		<Component
			{...omitCompatProps(rest)}
			className={cn(
				"leading-6",
				variant.startsWith("h") && "font-semibold tracking-tight",
				variant === "h6" && "font-medium tracking-[0.001em] leading-[1.6]",
				variant === "caption" && "leading-5",
				noWrap && "overflow-hidden text-ellipsis whitespace-nowrap",
				gutterBottom && "mb-2",
				className,
			)}
			style={{
				fontSize: fontSize ?? defaultSize,
				fontWeight,
				color: themeValue(color),
				textAlign,
				display,
				...sxToStyle(sx),
				...style,
			}}
		>
			{children}
		</Component>
	);
}

/** MUI-shaped field adapter used by screens that have not yet adopted TextField. */
interface LegacyTextFieldProps extends HTMLAttributes<HTMLInputElement> {
	value?: string | number;
	onChange?: ChangeEventHandler<HTMLInputElement | HTMLTextAreaElement>;
	label?: ReactNode;
	error?: boolean | string;
	helperText?: ReactNode;
	fullWidth?: boolean;
	multiline?: boolean;
	minRows?: number;
	maxRows?: number;
	inputProps?: Record<string, unknown>;
	slotProps?: Record<string, any>;
	InputProps?: Record<string, any>;
	type?: string;
	sx?: SxProps;
	[key: string]: any;
}

export function LegacyTextField({
	label,
	value,
	onChange,
	error,
	helperText,
	fullWidth,
	multiline,
	minRows,
	maxRows,
	inputProps,
	slotProps,
	InputProps,
	sx,
	className,
	type = "text",
	...props
}: LegacyTextFieldProps) {
	const generatedId = useId();
	const id = props.id ?? generatedId;
	const htmlInput = {
		...(inputProps ?? {}),
		...(slotProps?.htmlInput ?? {}),
	};
	const descriptionId = `${id}-description`;
	const invalid = typeof error === "string" ? error.length > 0 : Boolean(error);
	const controlProps = omitCompatProps(props);
	const legacyStyle = sxToStyle(sx);
	const htmlInputStyle =
		typeof htmlInput.style === "object" && htmlInput.style !== null
			? (htmlInput.style as CSSProperties)
			: undefined;
	const controlStyle = {
		...htmlInputStyle,
		height: htmlInputStyle?.height ?? legacyStyle.height,
	};
	delete controlProps.margin;
	return (
		<label
			className={cn(
				"flex min-w-0 flex-col gap-1",
				fullWidth && "w-full",
				className,
			)}
			style={legacyStyle}
			htmlFor={id}
		>
			{label && (
				<span className="text-xs font-semibold text-slate-200">{label}</span>
			)}
			{multiline ? (
				<textarea
					{...controlProps}
					{...htmlInput}
					id={id}
					value={value ?? ""}
					onChange={onChange}
					rows={minRows ?? 3}
					aria-invalid={invalid || undefined}
					aria-describedby={helperText ? descriptionId : undefined}
					className="min-h-9 min-w-0 rounded-md border border-ui-border bg-slate-950/65 px-3 py-2 text-sm text-fg outline-hidden transition placeholder:text-slate-600 focus:border-compat-primary focus-visible:outline-2 focus-visible:outline-focus disabled:cursor-not-allowed disabled:opacity-50"
					style={controlStyle}
				/>
			) : (
				<div className="relative flex items-center">
					{InputProps?.startAdornment}
					<input
						{...controlProps}
						{...htmlInput}
						id={id}
						type={type}
						value={value ?? ""}
						onChange={onChange}
						aria-invalid={invalid || undefined}
						aria-describedby={helperText ? descriptionId : undefined}
						className="h-9 min-w-0 flex-1 rounded-md border border-ui-border bg-slate-950/65 px-3 text-sm text-fg outline-hidden transition placeholder:text-slate-600 focus:border-compat-primary focus-visible:outline-2 focus-visible:outline-focus disabled:cursor-not-allowed disabled:opacity-50"
						style={controlStyle}
					/>
				</div>
			)}
			{helperText && (
				<span
					id={descriptionId}
					className={cn("text-xs", invalid ? "text-red-300" : "text-muted")}
				>
					{typeof helperText === "string" ? helperText : helperText}
				</span>
			)}
		</label>
	);
}

export function InputAdornment({ children, position: _position }: any) {
	return (
		<span className="inline-flex shrink-0 items-center px-2 text-muted">
			{children}
		</span>
	);
}

export function Paper({ children, variant, sx, className, ...props }: any) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(
				"rounded-md border border-ui-border bg-surface",
				variant === "outlined" ? "shadow-none" : "shadow-panel",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}

export function Card({ children, sx, className, ...props }: any) {
	return (
		<Paper {...props} sx={sx} className={className}>
			{children}
		</Paper>
	);
}

export function CardContent({ children, sx, className, ...props }: any) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn("p-4", className)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}

export function Container({
	children,
	maxWidth,
	sx,
	className,
	...props
}: any) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(
				"mx-auto w-full px-4 md:px-6",
				maxWidth === "xl" ? "max-w-[1536px]" : "max-w-6xl",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}

export function AppBar({ children, sx, className, ...props }: any) {
	return (
		<header
			{...omitCompatProps(props)}
			className={cn("shrink-0 bg-header text-white md:min-h-[69px]", className)}
			style={{
				boxShadow:
					"0 2px 4px -1px rgb(0 0 0 / 0.2), 0 4px 5px 0 rgb(0 0 0 / 0.14), 0 1px 10px 0 rgb(0 0 0 / 0.12)",
				...sxToStyle(sx),
			}}
		>
			{children}
		</header>
	);
}

export function Toolbar({ children, sx, className, ...props }: any) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn("flex min-h-14 items-center", className)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}

export function Divider({
	sx,
	className,
	orientation = "horizontal",
	...props
}: any) {
	return (
		<hr
			{...omitCompatProps(props)}
			className={cn(
				orientation === "vertical" ? "w-px self-stretch" : "h-px w-full",
				"bg-ui-border",
				className,
			)}
			style={sxToStyle(sx)}
		/>
	);
}

export function Avatar({ src, alt, sx, className, ...props }: any) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(
				"flex size-10 shrink-0 items-center justify-center overflow-hidden rounded-full",
				className,
			)}
			style={sxToStyle(sx)}
		>
			<img src={src} alt={alt ?? ""} className="size-full object-cover" />
		</div>
	);
}

export function List({ children, subheader, sx, className, ...props }: any) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn("flex flex-col", className)}
			style={sxToStyle(sx)}
		>
			{subheader}
			{children}
		</div>
	);
}

export function ListSubheader({ children, sx, className, ...props }: any) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(
				"px-4 py-2 text-xs font-semibold uppercase tracking-wider text-muted",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}

export function ListItem({
	children,
	sx,
	className,
	disablePadding,
	...props
}: any) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(!disablePadding && "px-4 py-1", className)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}

export function ListItemButton({
	children,
	sx,
	className,
	onClick,
	...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
	sx?: SxProps;
	[key: string]: any;
}) {
	return (
		<button
			type="button"
			{...omitCompatProps(props)}
			onClick={onClick}
			className={cn(
				"flex w-full cursor-pointer items-center gap-3 rounded px-3 py-2 text-left transition-colors hover:bg-slate-800 focus-visible:outline-2 focus-visible:outline-focus",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</button>
	);
}

export function ListItemIcon({ children, sx, className, ...props }: any) {
	return (
		<span
			{...omitCompatProps(props)}
			className={cn(
				"flex size-6 shrink-0 items-center justify-center",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</span>
	);
}

export function ListItemText({
	primary,
	secondary,
	children,
	sx,
	className,
	...props
}: any) {
	return (
		<span
			{...omitCompatProps(props)}
			className={cn("min-w-0 flex-1", className)}
			style={sxToStyle(sx)}
		>
			{children ?? (
				<>
					<span className="block truncate">{primary}</span>
					{secondary && (
						<span className="block text-xs text-muted">{secondary}</span>
					)}
				</>
			)}
		</span>
	);
}

export function Chip({
	label,
	children,
	color = "default",
	variant = "filled",
	size = "medium",
	sx,
	className,
	...props
}: any) {
	const tone =
		color === "error"
			? "border-red-400/50 bg-red-500/15 text-red-200"
			: color === "primary"
				? "border-compat-primary/50 bg-compat-primary/15 text-compat-primary"
				: "border-ui-border bg-slate-800 text-slate-200";
	return (
		<span
			{...omitCompatProps(props)}
			className={cn(
				"inline-flex items-center rounded-full border px-2 py-0.5 text-xs",
				variant === "outlined" && "bg-transparent",
				size === "small" && "text-[0.7rem]",
				tone,
				className,
			)}
			style={sxToStyle(sx)}
		>
			{label ?? children}
		</span>
	);
}

export function Alert({
	severity = "info",
	variant = "standard",
	action,
	icon,
	children,
	sx,
	className,
	...props
}: any) {
	const tone =
		severity === "error"
			? "border-red-400/40 bg-red-500/10 text-red-200"
			: severity === "warning"
				? "border-amber-400/40 bg-amber-500/10 text-amber-100"
				: severity === "success"
					? "border-emerald-400/40 bg-emerald-500/10 text-emerald-200"
					: "border-sky-400/40 bg-sky-500/10 text-sky-200";
	return (
		<div
			{...omitCompatProps(props)}
			role={severity === "error" ? "alert" : "status"}
			className={cn(
				"flex min-h-9 items-center gap-2 rounded-md border px-3 py-2 text-sm",
				tone,
				className,
			)}
			style={sxToStyle(sx)}
		>
			{icon}
			{!icon && (
				<span aria-hidden="true">
					{severity === "error" ? "!" : severity === "warning" ? "⚠" : "✓"}
				</span>
			)}
			<div className="min-w-0 flex-1">{children}</div>
			{action}
		</div>
	);
}

export function LinearProgress({
	value = 0,
	variant = "indeterminate",
	sx,
	className,
	...props
}: any) {
	return (
		<div
			{...omitCompatProps(props)}
			role="progressbar"
			aria-valuenow={variant === "determinate" ? value : undefined}
			className={cn(
				"h-1.5 w-full overflow-hidden rounded-full bg-slate-800",
				className,
			)}
			style={sxToStyle(sx)}
		>
			<div
				className={cn(
					"h-full rounded-full bg-compat-primary",
					variant !== "determinate" && "w-1/3 animate-pulse",
				)}
				style={
					variant === "determinate"
						? { width: `${Math.max(0, Math.min(100, value))}%` }
						: undefined
				}
			/>
		</div>
	);
}

export function CircularProgress({ size = 24, sx, className, ...props }: any) {
	return (
		<span
			{...omitCompatProps(props)}
			role="progressbar"
			className={cn(
				"inline-block animate-spin rounded-full border-2 border-current border-r-transparent",
				className,
			)}
			style={{ width: size, height: size, ...sxToStyle(sx) }}
		/>
	);
}

export function Collapse({ in: isIn, children, sx, className, ...props }: any) {
	return isIn ? (
		<div
			{...omitCompatProps(props)}
			className={className}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	) : null;
}

interface DialogContextValue {
	titleId: string;
}
const DialogContext = createContext<DialogContextValue | null>(null);

export function Dialog({
	open,
	onClose,
	children,
	fullWidth,
	maxWidth,
	sx,
	className,
	...props
}: any) {
	const titleId = useId();
	useEffect(() => {
		if (!open) return;
		const handleKeyDown = (event: KeyboardEvent) => {
			if (event.key === "Escape") onClose?.({}, "escapeKeyDown");
		};
		document.addEventListener("keydown", handleKeyDown);
		return () => document.removeEventListener("keydown", handleKeyDown);
	}, [open, onClose]);
	if (!open) return null;
	return (
		// biome-ignore lint/a11y/noStaticElementInteractions: the backdrop dismisses the controlled dialog.
		<div
			{...omitCompatProps(props)}
			className="fixed inset-0 z-50 flex items-center justify-center p-4"
			role="presentation"
			onMouseDown={(event) => {
				if (event.target === event.currentTarget)
					onClose?.({}, "backdropClick");
			}}
		>
			<div
				role="dialog"
				aria-modal="true"
				aria-labelledby={titleId}
				className={cn(
					"max-h-[calc(100vh-2rem)] w-full max-w-lg overflow-auto rounded-lg border border-ui-border bg-surface shadow-2xl",
					fullWidth && "max-w-2xl",
					className,
				)}
				style={sxToStyle(sx)}
			>
				<DialogContext.Provider value={{ titleId }}>
					{children}
				</DialogContext.Provider>
			</div>
		</div>
	);
}

export function DialogTitle({ children, id, sx, className, ...props }: any) {
	const context = useContext(DialogContext);
	return (
		<h2
			{...omitCompatProps(props)}
			id={id ?? context?.titleId}
			className={cn(
				"border-b border-ui-border px-5 py-4 text-lg font-semibold",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</h2>
	);
}

export function DialogContent({ children, sx, className, ...props }: any) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn("space-y-3 px-5 py-4", className)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}

export function DialogContentText({ children, sx, className, ...props }: any) {
	return (
		<p
			{...omitCompatProps(props)}
			className={cn("text-sm leading-6 text-slate-200", className)}
			style={sxToStyle(sx)}
		>
			{children}
		</p>
	);
}

export function DialogActions({ children, sx, className, ...props }: any) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(
				"flex justify-end gap-2 border-t border-ui-border px-5 py-3",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}

export function Drawer({
	open,
	onClose,
	anchor = "left",
	children,
	sx,
	className,
	...props
}: any) {
	const keepMounted = Boolean(props.ModalProps?.keepMounted);
	useEffect(() => {
		if (!open) return;
		const handler = (event: KeyboardEvent) =>
			event.key === "Escape" && onClose?.({}, "escapeKeyDown");
		document.addEventListener("keydown", handler);
		return () => document.removeEventListener("keydown", handler);
	}, [open, onClose]);
	if (!open && !keepMounted) return null;
	const edge = anchor === "right" ? "right-0" : "left-0";
	return (
		// biome-ignore lint/a11y/noStaticElementInteractions: the backdrop dismisses the controlled drawer.
		<div
			aria-hidden={!open || undefined}
			className={cn(
				"fixed inset-0 z-40",
				!open && "pointer-events-none invisible",
			)}
			role="presentation"
			onMouseDown={(event) => {
				if (event.target === event.currentTarget)
					onClose?.({}, "backdropClick");
			}}
		>
			<div
				{...omitCompatProps(props)}
				className={cn(
					"absolute bottom-0 top-0 w-80 max-w-[90vw] overflow-auto border-ui-border bg-surface shadow-2xl",
					edge,
					anchor === "left" ? "border-r" : "border-l",
					className,
				)}
				style={sxToStyle(sx)}
			>
				{children}
			</div>
		</div>
	);
}

export function Snackbar({
	open,
	children,
	anchorOrigin,
	sx,
	className,
	...props
}: any) {
	if (!open) return null;
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(
				"fixed bottom-4 left-1/2 z-[60] -translate-x-1/2",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}

export function Tooltip({ title, children }: any) {
	const [anchor, setAnchor] = useState<{ left: number; top: number } | null>(
		null,
	);
	const tooltipId = useId();
	if (!title) return children;

	const isTextTitle = typeof title === "string";
	const showTooltip = (
		target: EventTarget | null,
		clientX?: number,
		clientY?: number,
	) => {
		if (isTextTitle) return;
		const element = target instanceof HTMLElement ? target : null;
		const bounds = element?.getBoundingClientRect();
		setAnchor({
			left: clientX ?? bounds?.left ?? 0,
			top: clientY ?? bounds?.top ?? 0,
		});
	};

	return (
		<span
			id={isTextTitle ? undefined : tooltipId}
			title={isTextTitle ? title : undefined}
			className="contents"
			onPointerEnter={(event) =>
				showTooltip(event.target, event.clientX, event.clientY)
			}
			onPointerMove={(event) =>
				showTooltip(event.target, event.clientX, event.clientY)
			}
			onPointerLeave={() => setAnchor(null)}
			onFocusCapture={(event) => showTooltip(event.target)}
			onBlurCapture={() => setAnchor(null)}
			aria-describedby={!isTextTitle && anchor ? tooltipId : undefined}
		>
			{children}
			{!isTextTitle && anchor && (
				<span
					role="tooltip"
					className="pointer-events-none fixed z-[80] max-w-72 -translate-x-1/2 -translate-y-full rounded bg-slate-950 px-2 py-1 text-xs leading-4 text-slate-100 shadow-lg ring-1 ring-white/15"
					style={{ left: anchor.left, top: anchor.top - 8 }}
				>
					{title}
				</span>
			)}
		</span>
	);
}

interface MenuContextValue {
	inMenu: boolean;
}
const MenuContext = createContext<MenuContextValue>({ inMenu: false });

export function Menu({
	open,
	onClose,
	children,
	sx,
	className,
	...props
}: any) {
	useEffect(() => {
		if (!open) return;
		const handler = (event: KeyboardEvent) =>
			event.key === "Escape" && onClose?.(event);
		document.addEventListener("keydown", handler);
		return () => document.removeEventListener("keydown", handler);
	}, [open, onClose]);
	if (!open) return null;
	return (
		<div
			{...omitCompatProps(props)}
			role="menu"
			className={cn(
				"fixed right-4 top-16 z-50 min-w-40 rounded-md border border-ui-border bg-surface p-1 shadow-2xl",
				className,
			)}
			style={sxToStyle(sx)}
			onMouseDown={(event) => event.stopPropagation()}
		>
			<MenuContext.Provider value={{ inMenu: true }}>
				{children}
			</MenuContext.Provider>
		</div>
	);
}

export function MenuItem({
	children,
	value,
	onClick,
	sx,
	className,
	...props
}: any) {
	const { inMenu } = useContext(MenuContext);
	if (!inMenu && value !== undefined)
		return (
			<option
				value={value}
				className="bg-header text-fg"
				style={{
					backgroundColor: "var(--color-header)",
					color: "var(--color-fg)",
				}}
			>
				{children}
			</option>
		);
	if (!inMenu)
		return (
			<div
				{...omitCompatProps(props)}
				className={cn("flex items-center gap-0 px-4 py-1.5", className)}
				style={sxToStyle(sx)}
			>
				{children}
			</div>
		);
	return (
		<button
			type="button"
			role="menuitem"
			{...omitCompatProps(props)}
			onClick={onClick}
			className={cn(
				"block w-full rounded px-3 py-2 text-left text-sm hover:bg-slate-800 focus-visible:outline-2 focus-visible:outline-focus",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</button>
	);
}

export function Popover({
	open,
	onClose,
	children,
	sx,
	className,
	...props
}: any) {
	if (!open) return null;
	return (
		<div
			{...omitCompatProps(props)}
			role="dialog"
			className={cn(
				"fixed left-1/2 top-1/2 z-40 max-w-[calc(100vw-2rem)] -translate-x-1/2 -translate-y-1/2 rounded-md border border-ui-border bg-surface shadow-2xl",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}

export function FormControl({
	children,
	fullWidth,
	size,
	sx,
	className,
	...props
}: any) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(
				"relative flex min-w-0",
				fullWidth && "w-full",
				size === "small" && "max-h-9",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}

export function InputLabel({
	children,
	htmlFor,
	id,
	sx,
	className,
	...props
}: any) {
	return (
		<span
			{...omitCompatProps(props)}
			id={id}
			className={cn(
				"absolute left-0 top-0 z-10 bg-surface text-base font-normal leading-[1.4375] text-white/70",
				className,
			)}
			style={{
				transform: "translate(14px, -9px) scale(0.75)",
				transformOrigin: "top left",
				...sxToStyle(sx),
			}}
		>
			{children}
		</span>
	);
}

interface SelectContextValue {
	inSelect: boolean;
}
const SelectContext = createContext<SelectContextValue>({ inSelect: false });
export interface SelectChangeEvent<T = unknown>
	extends ChangeEvent<HTMLSelectElement> {
	target: EventTarget & { value: T } & HTMLSelectElement;
}

interface SelectProps extends HTMLAttributes<HTMLSelectElement> {
	value?: string | number;
	onChange?: (event: SelectChangeEvent) => void;
	label?: string;
	labelId?: string;
	size?: "small" | "medium";
	sx?: SxProps;
	[key: string]: any;
}

export function Select({
	value,
	onChange,
	children,
	label,
	labelId,
	size,
	sx,
	className,
	...props
}: SelectProps) {
	return (
		<SelectContext.Provider value={{ inSelect: true }}>
			<div className="relative w-full">
				<select
					{...omitCompatProps(props)}
					value={value}
					onChange={onChange}
					aria-label={label}
					aria-labelledby={labelId}
					className={cn(
						size === "small" ? "h-9" : "h-14",
						size === "small" ? "text-sm" : "text-base",
						"w-full appearance-none rounded border border-[rgba(255,255,255,0.23)] bg-header pl-[13px] pr-6 text-fg outline-hidden focus:border-compat-primary focus-visible:outline-2 focus-visible:outline-focus",
						className,
					)}
					style={{
						...sxToStyle(sx),
						colorScheme: "dark",
						backgroundImage:
							"url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='5' shape-rendering='crispEdges'%3E%3Cpath fill='%23939393' d='M0 0h10v1H0z M1 1h8v1H1z M2 2h6v1H2z M3 3h4v1H3z M4 4h2v1H4z'/%3E%3Cpath fill='%23fff' d='M1 0h8v1H1z M2 1h6v1H2z M3 2h4v1H3z M4 3h2v1H4z'/%3E%3C/svg%3E\")",
						backgroundPosition: "right 13px center",
						backgroundRepeat: "no-repeat",
						backgroundSize: "10px 5px",
					}}
				>
					{children}
				</select>
			</div>
		</SelectContext.Provider>
	);
}

export function FormControlLabel({
	control,
	label,
	value,
	sx,
	className,
	...props
}: any) {
	return (
		// biome-ignore lint/a11y/noLabelWithoutControl: the supplied control is rendered inside this label.
		<label
			{...omitCompatProps(props)}
			className={cn("inline-flex items-center gap-2", className)}
			style={sxToStyle(sx)}
		>
			{control}
			{label}
		</label>
	);
}

interface RadioContextValue {
	value: string;
	onChange?: (event: ChangeEvent<HTMLInputElement>) => void;
	name: string;
}
const RadioContext = createContext<RadioContextValue | null>(null);
export function RadioGroup({
	value,
	onChange,
	children,
	sx,
	className,
	name,
	...props
}: {
	value?: string;
	onChange?: ChangeEventHandler<HTMLInputElement>;
	children?: ReactNode;
	name?: string;
	sx?: SxProps;
	className?: string;
	[key: string]: any;
}) {
	const generated = useId();
	return (
		<RadioContext.Provider
			value={{ value: value ?? "", onChange, name: name ?? generated }}
		>
			<div
				{...omitCompatProps(props)}
				role="radiogroup"
				className={cn("flex flex-col gap-2", className)}
				style={sxToStyle(sx)}
			>
				{children}
			</div>
		</RadioContext.Provider>
	);
}

export function Radio({
	value,
	checked,
	onChange,
	disabled,
	size,
	sx,
	className,
	inputProps,
	...props
}: any) {
	const context = useContext(RadioContext);
	return (
		<input
			{...omitCompatProps(props)}
			{...inputProps}
			type="radio"
			name={context?.name}
			value={value}
			checked={checked ?? context?.value === value}
			onChange={onChange ?? context?.onChange}
			disabled={disabled}
			className={cn("size-4 accent-compat-primary", className)}
			style={sxToStyle(sx)}
		/>
	);
}

export function Switch({
	checked,
	onChange,
	disabled,
	sx,
	className,
	inputProps,
	...props
}: {
	checked?: boolean;
	onChange?: (event: ChangeEvent<HTMLInputElement>, checked: boolean) => void;
	disabled?: boolean;
	sx?: SxProps;
	className?: string;
	inputProps?: Record<string, unknown>;
	[key: string]: any;
}) {
	const handleChange = (event: ChangeEvent<HTMLInputElement>) =>
		onChange?.(event, event.currentTarget.checked);
	return (
		<label
			className={cn(
				"relative inline-flex h-5 w-9 shrink-0 items-center",
				disabled && "opacity-50",
				className,
			)}
			style={sxToStyle(sx)}
		>
			<input
				{...omitCompatProps(props)}
				{...inputProps}
				type="checkbox"
				role="switch"
				aria-checked={checked}
				checked={checked}
				onChange={handleChange}
				disabled={disabled}
				className="peer sr-only"
			/>
			<span
				aria-hidden="true"
				className="absolute inset-0 rounded-full bg-slate-700 transition-colors peer-checked:bg-compat-primary peer-focus-visible:outline-2 peer-focus-visible:outline-focus"
			/>
			<span
				aria-hidden="true"
				className="absolute left-0.5 size-4 rounded-full bg-white transition-transform peer-checked:translate-x-4"
			/>
		</label>
	);
}

interface SliderProps {
	value?: number | number[];
	min?: number;
	max?: number;
	step?: number;
	onChange?: (
		event: Event,
		value: number | number[],
		activeThumb: number,
	) => void;
	onChangeCommitted?: (event: Event, value: number | number[]) => void;
	onChangeEnd?: (value: number | number[]) => void;
	valueLabelFormat?: (value: number) => ReactNode;
	getAriaValueText?: (value: number) => string;
	sx?: SxProps;
	className?: string;
	orientation?: "horizontal" | "vertical";
	getAriaLabel?: (index: number) => string;
	"aria-label"?: string;
	disabled?: boolean;
	disableSwap?: boolean;
	[key: string]: any;
}

export function Slider({
	value,
	min = 0,
	max = 100,
	step = 1,
	onChange,
	onChangeCommitted,
	onChangeEnd,
	sx,
	className,
	orientation = "horizontal",
	getAriaLabel,
	getAriaValueText,
	"aria-label": ariaLabel,
	disabled,
	disableSwap = false,
	...props
}: SliderProps) {
	const values: number[] = Array.isArray(value)
		? value
		: [Number(value ?? min)];
	const range = values.length > 1;
	const rangeRef = useRef<HTMLDivElement | null>(null);
	const valuesRef = useRef(values);
	valuesRef.current = values;
	const activeThumbRef = useRef<number | null>(null);
	const [activeThumb, setActiveThumb] = useState<number | null>(null);
	const clamp = (next: number) => Math.min(max, Math.max(min, next));
	const snap = (next: number) => {
		if (!Number.isFinite(next)) return min;
		const steps = Math.round((next - min) / step);
		return clamp(min + steps * step);
	};
	const orderedValue = (index: number, nextValue: number) => {
		const next = [...valuesRef.current];
		next[index] = snap(nextValue);
		if (range && disableSwap) {
			if (index === 0) next[index] = Math.min(next[index], next[1]);
			else next[index] = Math.max(next[index], next[0]);
		}
		if (range && !disableSwap) next.sort((a, b) => a - b);
		return next;
	};
	const emitRangeChange = (index: number, next: number[], event: Event) => {
		valuesRef.current = next;
		onChange?.(event, range ? next : next[0], index);
	};
	const valueFromPointer = (clientX: number) => {
		const bounds = rangeRef.current?.getBoundingClientRect();
		if (!bounds || bounds.width <= 0) return min;
		return snap(min + ((clientX - bounds.left) / bounds.width) * (max - min));
	};
	const finishRangeChange = (event: Event) => {
		const next = valuesRef.current;
		onChangeCommitted?.(event, next);
		onChangeEnd?.(next);
		activeThumbRef.current = null;
		setActiveThumb(null);
	};
	const handleRangePointerDown = (
		index: number,
		event: React.PointerEvent<HTMLButtonElement>,
	) => {
		if (disabled) return;
		event.preventDefault();
		event.stopPropagation();
		activeThumbRef.current = index;
		setActiveThumb(index);
		event.currentTarget.setPointerCapture(event.pointerId);
		emitRangeChange(
			index,
			orderedValue(index, valueFromPointer(event.clientX)),
			event.nativeEvent,
		);
	};
	const handleRangePointerMove = (
		index: number,
		event: React.PointerEvent<HTMLButtonElement>,
	) => {
		if (activeThumbRef.current !== index) return;
		emitRangeChange(
			index,
			orderedValue(index, valueFromPointer(event.clientX)),
			event.nativeEvent,
		);
	};
	const handleRangePointerUp = (
		index: number,
		event: React.PointerEvent<HTMLButtonElement>,
	) => {
		if (activeThumbRef.current !== index) return;
		if (event.currentTarget.hasPointerCapture(event.pointerId)) {
			event.currentTarget.releasePointerCapture(event.pointerId);
		}
		finishRangeChange(event.nativeEvent);
	};
	const handleRangeKeyDown = (
		index: number,
		event: React.KeyboardEvent<HTMLButtonElement>,
	) => {
		if (disabled) return;
		let nextValue: number | null = null;
		const current = valuesRef.current[index] ?? min;
		if (event.key === "ArrowLeft" || event.key === "ArrowDown") {
			nextValue = current - step;
		} else if (event.key === "ArrowRight" || event.key === "ArrowUp") {
			nextValue = current + step;
		} else if (event.key === "PageDown") {
			nextValue = current - (max - min) / 10;
		} else if (event.key === "PageUp") {
			nextValue = current + (max - min) / 10;
		} else if (event.key === "Home") {
			nextValue = min;
		} else if (event.key === "End") {
			nextValue = max;
		}
		if (nextValue === null) return;
		event.preventDefault();
		const next = orderedValue(index, nextValue);
		emitRangeChange(index, next, event.nativeEvent);
	};
	const handleRangeTrackPointerDown = (
		event: React.PointerEvent<HTMLDivElement>,
	) => {
		if (disabled || event.target !== event.currentTarget) return;
		const pointerValue = valueFromPointer(event.clientX);
		const current = valuesRef.current;
		const index =
			Math.abs((current[0] ?? min) - pointerValue) <=
			Math.abs((current[1] ?? max) - pointerValue)
				? 0
				: 1;
		emitRangeChange(
			index,
			orderedValue(index, pointerValue),
			event.nativeEvent,
		);
	};
	if (range) {
		const toPercent = (current: number) =>
			((clamp(current) - min) / Math.max(1, max - min)) * 100;
		const lower = toPercent(values[0] ?? min);
		const upper = toPercent(values[1] ?? max);
		return (
			<div
				ref={rangeRef}
				className={cn(
					"relative flex min-h-6 w-full touch-none items-center",
					disabled && "opacity-50",
					orientation === "vertical" && "h-32 w-6",
					className,
				)}
				style={sxToStyle(sx)}
				onPointerDown={handleRangeTrackPointerDown}
			>
				<span
					aria-hidden="true"
					className="pointer-events-none absolute left-0 right-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-slate-700"
				/>
				<span
					aria-hidden="true"
					className="pointer-events-none absolute top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-compat-primary"
					style={{ left: `${lower}%`, width: `${Math.max(0, upper - lower)}%` }}
				/>
				{values.map((current, index) => (
					<button
						key={index}
						type="button"
						role="slider"
						aria-label={
							getAriaLabel?.(index) ??
							ariaLabel ??
							(index === 0 ? "Minimum" : "Maximum")
						}
						aria-valuemin={min}
						aria-valuemax={max}
						aria-valuenow={current}
						aria-valuetext={getAriaValueText?.(current)}
						disabled={disabled}
						className={cn(
							"absolute top-1/2 z-10 h-[25px] w-[5px] -translate-x-1/2 -translate-y-1/2 cursor-ew-resize rounded-[1px] border-0 bg-compat-primary p-0 shadow-[0_1px_3px_rgba(2,6,23,0.7)] outline-hidden focus-visible:outline-2 focus-visible:outline-focus",
							activeThumb === index && "z-20",
						)}
						style={{ left: `${toPercent(current)}%` }}
						onPointerDown={(event) => handleRangePointerDown(index, event)}
						onPointerMove={(event) => handleRangePointerMove(index, event)}
						onPointerUp={(event) => handleRangePointerUp(index, event)}
						onPointerCancel={(event) => handleRangePointerUp(index, event)}
						onKeyDown={(event) => handleRangeKeyDown(index, event)}
						onKeyUp={(event) => {
							if (
								event.key === "ArrowLeft" ||
								event.key === "ArrowRight" ||
								event.key === "ArrowUp" ||
								event.key === "ArrowDown" ||
								event.key === "Home" ||
								event.key === "End" ||
								event.key === "PageUp" ||
								event.key === "PageDown"
							) {
								finishRangeChange(event.nativeEvent);
							}
						}}
					/>
				))}
			</div>
		);
	}
	const emit = (index: number, event: ChangeEvent<HTMLInputElement>) => {
		const next = [...values];
		next[index] = Number(event.currentTarget.value);
		if (range) next.sort((a, b) => a - b);
		onChange?.(event.nativeEvent, range ? next : next[0], index);
	};
	const inputs = values.map((current, index) => (
		<input
			key={index}
			{...omitCompatProps(props)}
			type="range"
			min={min}
			max={max}
			step={step}
			value={current}
			disabled={disabled}
			aria-label={
				range
					? (getAriaLabel?.(index) ?? (index === 0 ? "Minimum" : "Maximum"))
					: (getAriaLabel?.(index) ?? ariaLabel)
			}
			aria-valuetext={getAriaValueText?.(current)}
			onChange={(event) => emit(index, event)}
			onMouseUp={(event) => {
				const target = event.currentTarget;
				const next = range ? values : values[0];
				onChangeCommitted?.(event.nativeEvent, range ? next : next);
				onChangeEnd?.(range ? values : values[0]);
				target.blur();
			}}
			className={cn(
				"h-1.5 w-full accent-compat-primary",
				range && "absolute inset-0",
				orientation === "vertical" && "rotate-[-90deg]",
				className,
			)}
		/>
	));
	return (
		<div
			className={cn(
				"relative flex min-h-6 w-full items-center",
				orientation === "vertical" && "h-32 w-6",
			)}
			style={sxToStyle(sx)}
		>
			{inputs}
		</div>
	);
}

export function createTheme(theme: Record<string, unknown>) {
	return theme;
}

export function Tabs({
	value,
	onChange,
	children,
	variant,
	sx,
	className,
	...props
}: {
	value?: unknown;
	onChange?: (event: SyntheticEvent, value: any) => void;
	children?: ReactNode;
	variant?: string;
	sx?: SxProps;
	className?: string;
	[key: string]: any;
}) {
	return (
		<div
			{...omitCompatProps(props)}
			role="tablist"
			className={cn(
				"flex min-h-10 border-b border-ui-border",
				variant === "fullWidth" && "w-full",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{Children.map(children, (child) =>
				isValidElement(child)
					? cloneElement(child as ReactElement<any>, {
							__tabsValue: value,
							__tabsOnChange: onChange,
							__tabsFullWidth: variant === "fullWidth",
						})
					: child,
			)}
		</div>
	);
}

export function Tab({
	label,
	value,
	__tabsValue,
	__tabsOnChange,
	__tabsFullWidth,
	children,
	sx,
	className,
	...props
}: any) {
	const selected = __tabsValue === value;
	return (
		<button
			type="button"
			role="tab"
			aria-selected={selected}
			{...omitCompatProps(props)}
			onClick={(event) => __tabsOnChange?.(event, value)}
			className={cn(
				"min-h-10 border-b-2 border-transparent px-3 py-2 text-sm text-muted transition-colors hover:text-fg",
				selected && "border-compat-primary text-compat-primary",
				__tabsFullWidth && "flex-1",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{label ?? children}
		</button>
	);
}

interface AccordionContextValue {
	expanded: boolean;
	toggle: () => void;
}
const AccordionContext = createContext<AccordionContextValue | null>(null);
export function Accordion({
	expanded: controlled,
	defaultExpanded = false,
	onChange,
	children,
	sx,
	className,
	...props
}: any) {
	const [internal, setInternal] = useState(defaultExpanded);
	const expanded = controlled ?? internal;
	const toggle = () => {
		const next = !expanded;
		if (controlled === undefined) setInternal(next);
		onChange?.({}, next);
	};
	return (
		<AccordionContext.Provider value={{ expanded, toggle }}>
			<div
				{...omitCompatProps(props)}
				className={cn(
					"overflow-hidden rounded-md border border-ui-border",
					className,
				)}
				style={sxToStyle(sx)}
			>
				{children}
			</div>
		</AccordionContext.Provider>
	);
}
export function AccordionSummary({
	children,
	expandIcon,
	sx,
	className,
	...props
}: any) {
	const context = useContext(AccordionContext);
	return (
		<button
			type="button"
			{...omitCompatProps(props)}
			aria-expanded={context?.expanded}
			onClick={context?.toggle}
			className={cn(
				"flex w-full items-center justify-between px-4 py-3 text-left hover:bg-slate-800/50",
				className,
			)}
			style={sxToStyle(sx)}
		>
			<span className="min-w-0 flex-1">{children}</span>
			{expandIcon ?? (
				<span aria-hidden="true" className="text-muted">
					⌄
				</span>
			)}
		</button>
	);
}
export function AccordionDetails({ children, sx, className, ...props }: any) {
	const context = useContext(AccordionContext);
	return context?.expanded ? (
		<div
			{...omitCompatProps(props)}
			className={cn("border-t border-ui-border px-4 py-3", className)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	) : null;
}

export function Grid({
	container,
	children,
	size,
	spacing,
	sx,
	className,
	...props
}: any) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(container && "grid", className)}
			style={{
				...(container
					? {
							gap: spacingValue(spacing),
							gridTemplateColumns: "repeat(auto-fit, minmax(180px, 1fr))",
						}
					: {}),
				...sxToStyle(sx),
			}}
		>
			{children}
		</div>
	);
}

export function Link({ children, sx, className, ...props }: any) {
	return (
		<a
			{...omitCompatProps(props)}
			className={cn(
				"text-cyan-300 underline-offset-2 hover:underline",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</a>
	);
}

export function styled(Component: any) {
	return (
		styles:
			| Record<string, unknown>
			| ((props: any) => Record<string, unknown>)
			| TemplateStringsArray,
	) =>
		(props: any) => (
			<Component
				{...props}
				sx={{
					...(Array.isArray(styles)
						? {}
						: typeof styles === "function"
							? styles(props)
							: styles),
					...props.sx,
				}}
			/>
		);
}

export function keyframes(
	frames: TemplateStringsArray | Record<string, unknown>,
) {
	return Array.isArray(frames) ? frames.join("") : JSON.stringify(frames);
}

export function useTheme() {
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

export function useMediaQuery(query: string) {
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

export function CssBaseline() {
	return null;
}
export function GlobalStyles(_props: any) {
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

interface SimpleTreeViewProps {
	expandedItems?: string[];
	selectedItems?: string | null;
	onExpandedItemsChange?: (
		event: SyntheticEvent | null,
		itemIds: string[],
	) => void;
	onSelectedItemsChange?: (
		event: SyntheticEvent | null,
		itemId: string | null,
	) => void;
	onItemClick?: (event: SyntheticEvent | null, itemId: string) => void;
	children?: ReactNode;
	className?: string;
	[key: string]: any;
}

export function SimpleTreeView(props: SimpleTreeViewProps) {
	const expanded = new Set<string>(props.expandedItems ?? []);
	const context: TreeContextValue = {
		expanded,
		selected: props.selectedItems ?? null,
		toggle: (id) =>
			props.onExpandedItemsChange?.(null, toggleId([...expanded], id)),
		select: (id) => {
			props.onSelectedItemsChange?.(null, id);
			props.onItemClick?.(null, id);
		},
	};
	return (
		<TreeContext.Provider value={context}>
			<TreeLevelContext.Provider value={{ depth: 0, isLast: true }}>
				<div
					role="tree"
					{...omitCompatProps(props)}
					className={cn("w-full", props.className)}
				>
					{props.children}
				</div>
			</TreeLevelContext.Provider>
		</TreeContext.Provider>
	);
}

interface TreeContextValue {
	expanded: Set<string>;
	selected: string | null;
	toggle: (id: string) => void;
	select: (id: string) => void;
}
const TreeContext = createContext<TreeContextValue | null>(null);
const TreeLevelContext = createContext({ depth: 0, isLast: true });
export function TreeItem({
	itemId,
	label,
	children,
	className,
	sx,
	...props
}: any) {
	const parent = useContext(TreeContext);
	const { depth, isLast } = useContext(TreeLevelContext);
	const expanded = parent?.expanded.has(itemId) ?? false;
	const selected = parent?.selected === itemId;
	const childItems = Children.toArray(children);
	const hasChildren = childItems.length > 0;
	const handleRowClick = () => {
		if (hasChildren) parent?.toggle(itemId);
		else parent?.select(itemId);
	};
	const handleKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
		if (event.key === "Enter" || event.key === " ") {
			event.preventDefault();
			handleRowClick();
			return;
		}
		if (!hasChildren) return;
		if (event.key === "ArrowRight" && !expanded) {
			event.preventDefault();
			parent?.toggle(itemId);
		} else if (event.key === "ArrowLeft" && expanded) {
			event.preventDefault();
			parent?.toggle(itemId);
		}
	};
	return (
		<div
			{...omitCompatProps(props)}
			role="treeitem"
			tabIndex={0}
			aria-expanded={hasChildren ? expanded : undefined}
			aria-selected={selected}
			onKeyDown={handleKeyDown}
			className={cn("relative", className)}
			style={sxToStyle(sx)}
		>
			{depth > 0 && (
				<>
					<span
						aria-hidden="true"
						className="pointer-events-none absolute -left-2 top-[18px] w-2 border-t-2 border-ui-border"
					/>
					{!isLast && (
						<span
							aria-hidden="true"
							className="pointer-events-none absolute -bottom-[22px] -left-2 top-[18px] border-l-2 border-ui-border"
						/>
					)}
				</>
			)}
			<div
				className={cn(
					"relative flex min-h-9 w-full cursor-pointer items-center rounded-md border border-transparent bg-surface-raised px-2 text-left text-sm text-fg transition-colors hover:bg-header focus-within:outline-2 focus-within:outline-compat-primary focus-within:outline-offset-1",
					selected &&
						"border-compat-primary/55 bg-ui-border text-fg shadow-panel",
				)}
				onClick={(event) => {
					event.stopPropagation();
					handleRowClick();
				}}
			>
				{hasChildren && (
					<button
						type="button"
						aria-label={expanded ? "Collapse" : "Expand"}
						onClick={(event) => {
							event.stopPropagation();
							parent?.toggle(itemId);
						}}
						className="mr-1 inline-flex size-5 shrink-0 items-center justify-center rounded text-fg hover:bg-ui-border"
					>
						{expanded ? "▾" : "▸"}
					</button>
				)}
				<div className="min-w-0 flex-1 truncate text-left">{label}</div>
			</div>
			{hasChildren && expanded && (
				<div className="relative ml-3 space-y-1 pl-2 pt-1">
					<span
						aria-hidden="true"
						className="pointer-events-none absolute left-0 top-0 h-[22px] border-l-2 border-ui-border"
					/>
					{childItems.map((child, index) => (
						<TreeLevelContext.Provider
							key={index}
							value={{
								depth: depth + 1,
								isLast: index === childItems.length - 1,
							}}
						>
							{child}
						</TreeLevelContext.Provider>
					))}
				</div>
			)}
		</div>
	);
}

export const treeItemClasses = {
	content: "tree-item-content",
	selected: "tree-item-selected",
	iconContainer: "tree-item-icon",
};

function toggleId(ids: string[], id: string) {
	return ids.includes(id) ? ids.filter((item) => item !== id) : [...ids, id];
}
