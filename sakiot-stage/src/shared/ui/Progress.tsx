import type { HTMLAttributes } from "react";
import { cn } from "./cn";
import { omitCompatProps, type SxProps, sxToStyle } from "./theme";

export interface LinearProgressProps extends HTMLAttributes<HTMLDivElement> {
	value?: number;
	variant?: "determinate" | "indeterminate";
	sx?: SxProps;
	[key: string]: any;
}

export function LinearProgress({
	value = 0,
	variant = "indeterminate",
	sx,
	className,
	...props
}: LinearProgressProps) {
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

export interface CircularProgressProps extends HTMLAttributes<HTMLSpanElement> {
	size?: number | string;
	sx?: SxProps;
	[key: string]: any;
}

export function CircularProgress({
	size = 24,
	sx,
	className,
	...props
}: CircularProgressProps) {
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
