import type { RefAttributes } from "react";
import {
	Menu as AriaMenu,
	MenuItem as AriaMenuItem,
	MenuSection as AriaMenuSection,
	composeRenderProps,
	type MenuItemProps,
	type MenuProps,
	type MenuSectionProps,
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
/** Groups menu items; a section can carry its own selection state. */
export function MenuSection<T extends object>({
	className,
	...props
}: MenuSectionProps<T> & RefAttributes<HTMLElement>) {
	return (
		<AriaMenuSection
			{...props}
			className={cn(
				"mt-1 border-t border-ui-border pt-1 first:mt-0 first:border-t-0 first:pt-0",
				className,
			)}
		/>
	);
}
