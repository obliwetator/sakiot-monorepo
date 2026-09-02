import type { HTMLAttributes, ReactNode } from "react";
import { cn } from "./cn";
import { omitCompatProps, type SxProps, sxToStyle } from "./theme";

export interface ChipProps extends HTMLAttributes<HTMLSpanElement> {
	label?: ReactNode;
	color?:
		| "default"
		| "primary"
		| "secondary"
		| "error"
		| "warning"
		| "success"
		| "info";
	variant?: "filled" | "outlined";
	size?: "small" | "medium";
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
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
}: ChipProps) {
	const tone =
		color === "error"
			? "border-red-400/50 bg-red-500/15 text-red-200"
			: color === "primary"
				? "border-compat-primary/50 bg-compat-primary/15 text-compat-primary"
				: color === "secondary"
					? "border-purple-400/50 bg-purple-500/15 text-purple-200"
					: color === "warning"
						? "border-amber-400/50 bg-amber-500/15 text-amber-200"
						: color === "success"
							? "border-emerald-400/50 bg-emerald-500/15 text-emerald-200"
							: "border-ui-border bg-slate-800 text-slate-200";
	return (
		<span
			{...omitCompatProps(props)}
			className={cn(
				"inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium",
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

export interface AvatarProps extends HTMLAttributes<HTMLDivElement> {
	src?: string;
	alt?: string;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function Avatar({
	src,
	alt,
	children,
	sx,
	className,
	...props
}: AvatarProps) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(
				"relative flex size-10 shrink-0 items-center justify-center overflow-hidden rounded-full bg-slate-700 text-sm font-semibold text-slate-100",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{src ? (
				<img src={src} alt={alt} className="size-full object-cover" />
			) : (
				children
			)}
		</div>
	);
}
