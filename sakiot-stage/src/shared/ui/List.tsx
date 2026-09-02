import type { ButtonHTMLAttributes, HTMLAttributes, ReactNode } from "react";
import { cn } from "./cn";
import { omitCompatProps, type SxProps, sxToStyle } from "./theme";

export interface ListProps extends HTMLAttributes<HTMLDivElement> {
	subheader?: ReactNode;
	sx?: SxProps;
	children?: ReactNode;
	dense?: boolean;
	disablePadding?: boolean;
	[key: string]: any;
}

export function List({
	children,
	subheader,
	dense: _dense,
	disablePadding: _disablePadding,
	sx,
	className,
	...props
}: ListProps) {
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

export function ListSubheader({
	children,
	sx,
	className,
	...props
}: HTMLAttributes<HTMLDivElement> & { sx?: SxProps; [key: string]: any }) {
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

export interface ListItemProps extends HTMLAttributes<HTMLDivElement> {
	disablePadding?: boolean;
	secondaryAction?: ReactNode;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function ListItem({
	children,
	sx,
	className,
	disablePadding,
	secondaryAction,
	...props
}: ListItemProps) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(
				"relative flex items-center",
				!disablePadding && "px-4 py-1",
				secondaryAction && "justify-between gap-2",
				className,
			)}
			style={sxToStyle(sx)}
		>
			<div className="min-w-0 flex-1">{children}</div>
			{secondaryAction && (
				<div className="shrink-0 pl-2">{secondaryAction}</div>
			)}
		</div>
	);
}

export interface ListItemButtonProps
	extends ButtonHTMLAttributes<HTMLButtonElement> {
	selected?: boolean;
	dense?: boolean;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function ListItemButton({
	children,
	sx,
	className,
	onClick,
	selected,
	dense,
	...props
}: ListItemButtonProps) {
	return (
		<button
			type="button"
			{...omitCompatProps(props)}
			onClick={onClick}
			className={cn(
				"flex w-full cursor-pointer items-center gap-3 rounded px-3 py-2 text-left transition-colors hover:bg-slate-800 focus-visible:outline-2 focus-visible:outline-focus",
				dense && "py-1 text-sm",
				selected && "bg-slate-800 font-medium text-fg",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</button>
	);
}

export interface ListItemIconProps extends HTMLAttributes<HTMLSpanElement> {
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function ListItemIcon({
	children,
	sx,
	className,
	...props
}: ListItemIconProps) {
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

export interface ListItemTextProps extends HTMLAttributes<HTMLSpanElement> {
	primary?: ReactNode;
	secondary?: ReactNode;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function ListItemText({
	primary,
	secondary,
	children,
	sx,
	className,
	...props
}: ListItemTextProps) {
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
