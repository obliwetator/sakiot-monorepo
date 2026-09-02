import type { HTMLAttributes, ReactNode } from "react";
import { cn } from "./cn";
import {
	omitCompatProps,
	type SxProps,
	spacingValue,
	sxToStyle,
} from "./theme";

export interface PaperProps extends HTMLAttributes<HTMLDivElement> {
	variant?: "elevation" | "outlined";
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function Paper({
	children,
	variant,
	sx,
	className,
	...props
}: PaperProps) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(
				"rounded-md border border-ui-border bg-surface text-fg shadow-sm",
				variant === "outlined" && "border-ui-border shadow-none",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}

export function Card({ children, sx, className, ...props }: PaperProps) {
	return (
		<Paper {...props} sx={sx} className={className}>
			{children}
		</Paper>
	);
}

export function CardContent({
	children,
	sx,
	className,
	...props
}: HTMLAttributes<HTMLDivElement> & { sx?: SxProps; [key: string]: any }) {
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
	maxWidth = "lg",
	sx,
	className,
	...props
}: HTMLAttributes<HTMLDivElement> & {
	maxWidth?: string;
	sx?: SxProps;
	[key: string]: any;
}) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(
				"mx-auto w-full px-4 sm:px-6",
				maxWidth === "lg" && "max-w-7xl",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}

export function AppBar({
	children,
	sx,
	className,
	...props
}: HTMLAttributes<HTMLElement> & { sx?: SxProps; [key: string]: any }) {
	return (
		<header
			{...omitCompatProps(props)}
			className={cn(
				"w-full border-b border-ui-border bg-header text-fg shadow-sm",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</header>
	);
}

export function Toolbar({
	children,
	sx,
	className,
	...props
}: HTMLAttributes<HTMLDivElement> & { sx?: SxProps; [key: string]: any }) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(
				"flex min-h-14 items-center justify-between gap-4 px-4 sm:min-h-16",
				className,
			)}
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
}: HTMLAttributes<HTMLHRElement> & {
	orientation?: "horizontal" | "vertical";
	sx?: SxProps;
	[key: string]: any;
}) {
	return (
		<hr
			{...omitCompatProps(props)}
			className={cn(
				"border-ui-border",
				orientation === "vertical" ? "h-full w-px border-l" : "w-full border-t",
				className,
			)}
			style={sxToStyle(sx)}
		/>
	);
}

export function Grid({
	container,
	children,
	size: _size,
	spacing,
	sx,
	className,
	...props
}: HTMLAttributes<HTMLDivElement> & {
	container?: boolean;
	spacing?: unknown;
	size?: unknown;
	sx?: SxProps;
	[key: string]: any;
}) {
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
