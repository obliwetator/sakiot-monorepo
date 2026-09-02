import {
	type ButtonHTMLAttributes,
	Children,
	cloneElement,
	type HTMLAttributes,
	isValidElement,
	type ReactElement,
	type ReactNode,
	type SyntheticEvent,
} from "react";
import { cn } from "./cn";
import { omitCompatProps, type SxProps, sxToStyle } from "./theme";

export interface TabsProps
	extends Omit<HTMLAttributes<HTMLDivElement>, "onChange"> {
	value?: unknown;
	onChange?: (event: SyntheticEvent, value: any) => void;
	children?: ReactNode;
	variant?: string;
	sx?: SxProps;
	[key: string]: any;
}

export function Tabs({
	value,
	onChange,
	children,
	variant,
	sx,
	className,
	...props
}: TabsProps) {
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

export interface TabProps
	extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "value"> {
	label?: ReactNode;
	value?: unknown;
	__tabsValue?: unknown;
	__tabsOnChange?: (event: SyntheticEvent, value: any) => void;
	__tabsFullWidth?: boolean;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
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
}: TabProps) {
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
