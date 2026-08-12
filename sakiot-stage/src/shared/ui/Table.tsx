import type { ComponentPropsWithoutRef } from "react";
import { cn } from "./cn";

export function TableContainer({
	className,
	...props
}: ComponentPropsWithoutRef<"div">) {
	return <div className={cn("w-full overflow-x-auto", className)} {...props} />;
}

export function Table({
	className,
	...props
}: ComponentPropsWithoutRef<"table">) {
	return (
		<table
			className={cn(
				"w-full min-w-160 border-collapse text-left text-sm",
				className,
			)}
			{...props}
		/>
	);
}

export const TableHeader = (props: ComponentPropsWithoutRef<"thead">) => (
	<thead {...props} />
);
export const TableBody = (props: ComponentPropsWithoutRef<"tbody">) => (
	<tbody {...props} />
);

export function TableRow({
	className,
	...props
}: ComponentPropsWithoutRef<"tr">) {
	return (
		<tr
			className={cn("border-b border-slate-800 last:border-b-0", className)}
			{...props}
		/>
	);
}

export function TableHead({
	className,
	...props
}: ComponentPropsWithoutRef<"th">) {
	return (
		<th
			className={cn(
				"bg-slate-950/45 px-3 py-2.5 text-xs font-semibold uppercase tracking-wider text-muted",
				className,
			)}
			{...props}
		/>
	);
}

export function TableCell({
	className,
	...props
}: ComponentPropsWithoutRef<"td">) {
	return (
		<td className={cn("px-3 py-3 text-slate-200", className)} {...props} />
	);
}
