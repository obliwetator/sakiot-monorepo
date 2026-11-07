import type { ComponentProps } from "react";
import { cn } from "./cn";
export function Table({ className, ...props }: ComponentProps<"table">) {
	return (
		<table
			{...props}
			className={cn(
				"w-full min-w-160 border-collapse text-left text-sm",
				className,
			)}
		/>
	);
}
/**
 * Opaque, because in a ScrollRegion the rows scroll underneath it. It sticks a
 * pixel past the edge so that rounding at fractional positions leaves no seam.
 */
export function TableHeader({ className, ...props }: ComponentProps<"thead">) {
	return (
		<thead
			{...props}
			className={cn("sticky -top-px z-1 bg-surface", className)}
		/>
	);
}
export function TableBody(props: ComponentProps<"tbody">) {
	return <tbody {...props} />;
}
export function TableRow({ className, ...props }: ComponentProps<"tr">) {
	return (
		<tr
			{...props}
			className={cn("border-b border-slate-800 last:border-b-0", className)}
		/>
	);
}
export function TableHead({ className, ...props }: ComponentProps<"th">) {
	return (
		<th
			{...props}
			className={cn(
				"bg-slate-950/45 px-3 py-2.5 text-xs font-semibold uppercase tracking-wider text-muted",
				className,
			)}
		/>
	);
}
export function TableCell({ className, ...props }: ComponentProps<"td">) {
	return (
		<td {...props} className={cn("px-3 py-3 text-slate-200", className)} />
	);
}
