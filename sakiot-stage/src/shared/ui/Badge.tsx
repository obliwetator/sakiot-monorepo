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
	const isSmall = size === "small";
	const isOutlined = variant === "outlined";

	const colorClasses: Record<string, string> = {
		default: isOutlined
			? "border-white/20 bg-transparent text-slate-200"
			: "border-transparent bg-white/10 text-slate-200",
		primary: isOutlined
			? "border-compat-primary/60 bg-transparent text-compat-primary"
			: "border-transparent bg-compat-primary text-slate-950 font-medium",
		secondary: isOutlined
			? "border-[#a78bfa]/60 bg-transparent text-[#c4b5fd]"
			: "border-transparent bg-[#a78bfa] text-slate-950 font-medium",
		error: isOutlined
			? "border-red-400/60 bg-transparent text-red-300"
			: "border-transparent bg-red-500/20 text-red-200",
		warning: isOutlined
			? "border-amber-400/60 bg-transparent text-amber-300"
			: "border-transparent bg-amber-500/20 text-amber-200",
		success: isOutlined
			? "border-emerald-400/60 bg-transparent text-emerald-300"
			: "border-transparent bg-emerald-500/20 text-emerald-200",
		info: isOutlined
			? "border-sky-400/60 bg-transparent text-sky-300"
			: "border-transparent bg-sky-500/20 text-sky-200",
	};

	return (
		<span
			{...omitCompatProps(props)}
			className={cn(
				"inline-flex shrink-0 self-center items-center justify-center rounded-full border box-border whitespace-nowrap text-center font-normal transition-colors select-none",
				isSmall
					? "h-6 min-h-6 max-h-6 px-2 text-xs leading-none"
					: "h-8 min-h-8 max-h-8 px-3 text-xs leading-none",
				colorClasses[color] ?? colorClasses.default,
				className,
			)}
			style={sxToStyle(sx)}
		>
			<span className="truncate">{label ?? children}</span>
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
