import type { RefAttributes } from "react";
import {
	Menu as AriaMenu,
	MenuItem as AriaMenuItem,
	composeRenderProps,
	type MenuItemProps,
	type MenuProps,
	MenuTrigger,
	Popover,
} from "react-aria-components";
import { cn } from "./cn";

export { MenuTrigger, Popover };
export function Menu<T extends object>({
	className,
	...props
}: MenuProps<T> & RefAttributes<HTMLDivElement>) {
	return (
		<AriaMenu
			{...props}
			className={composeRenderProps(className, (className) =>
				cn(
					"min-w-40 rounded-md border border-ui-border bg-surface p-1 text-fg shadow-2xl outline-hidden",
					className,
				),
			)}
		/>
	);
}
export function MenuItem<T extends object>({
	className,
	...props
}: MenuItemProps<T> & RefAttributes<HTMLDivElement>) {
	return (
		<AriaMenuItem
			{...props}
			className={composeRenderProps(className, (className) =>
				cn(
					"cursor-default rounded px-3 py-2 text-sm outline-hidden data-[focused]:bg-slate-800 data-[disabled]:opacity-50",
					className,
				),
			)}
		/>
	);
}
