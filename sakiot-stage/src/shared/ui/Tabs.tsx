import type { RefAttributes } from "react";
import { composeRenderProps } from "react-aria-components";
import {
	Tab as AriaTab,
	TabList as AriaTabList,
	type TabListProps,
	TabPanel,
	type TabProps,
	Tabs,
} from "react-aria-components/Tabs";
import { cn } from "./cn";

export { TabPanel, Tabs };
export function TabList<T extends object>({
	className,
	...props
}: TabListProps<T> & RefAttributes<HTMLDivElement>) {
	return (
		<AriaTabList
			{...props}
			className={composeRenderProps(className, (className) =>
				cn("flex min-h-10 border-b border-ui-border", className),
			)}
		/>
	);
}
export function Tab({
	className,
	...props
}: TabProps & RefAttributes<HTMLDivElement>) {
	return (
		<AriaTab
			{...props}
			className={composeRenderProps(className, (className) =>
				cn(
					"min-h-10 cursor-default border-b-2 border-transparent px-3 py-2 text-sm text-muted outline-hidden transition-colors data-[hovered]:text-fg data-[selected]:border-accent data-[selected]:text-accent data-[focus-visible]:outline-2 data-[focus-visible]:outline-focus",
					className,
				),
			)}
		/>
	);
}
