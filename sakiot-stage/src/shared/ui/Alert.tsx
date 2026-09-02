import type { HTMLAttributes, ReactNode } from "react";
import { cn } from "./cn";
import { omitCompatProps, type SxProps, sxToStyle } from "./theme";

export interface AlertProps extends HTMLAttributes<HTMLDivElement> {
	severity?: "error" | "warning" | "success" | "info";
	variant?: "standard" | "outlined" | "filled";
	action?: ReactNode;
	icon?: ReactNode;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function Alert({
	severity = "info",
	variant: _variant = "standard",
	action,
	icon,
	children,
	sx,
	className,
	...props
}: AlertProps) {
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

export function Snackbar({
	open,
	children,
	anchorOrigin: _anchorOrigin,
	sx,
	className,
	...props
}: HTMLAttributes<HTMLDivElement> & {
	open?: boolean;
	anchorOrigin?: unknown;
	sx?: SxProps;
	[key: string]: any;
}) {
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
