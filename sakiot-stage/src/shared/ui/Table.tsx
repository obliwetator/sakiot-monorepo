import type { ComponentPropsWithoutRef } from "react";
import { cn } from "./cn";

export function TableContainer({
	className,
	component: Component = "div",
	variant: _variant,
	sx,
	children,
	...props
}: any) {
	return (
		<Component
			className={cn("w-full overflow-x-auto", className)}
			style={sx}
			{...props}
		>
			{children}
		</Component>
	);
}

export function Table({ className, size: _size, sx, children, ...props }: any) {
	return (
		<table
			className={cn(
				"w-full min-w-160 border-collapse text-left text-sm",
				className,
			)}
			style={sx}
			{...props}
		>
			{children}
		</table>
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
	hover: _hover,
	sx,
	children,
	...props
}: any) {
	return (
		<tr
			className={cn("border-b border-slate-800 last:border-b-0", className)}
			style={sx}
			{...props}
		>
			{children}
		</tr>
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
