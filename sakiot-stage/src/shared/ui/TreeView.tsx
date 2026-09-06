import type { ComponentProps, RefAttributes } from "react";
import { composeRenderProps } from "react-aria-components";
import {
	Tree as AriaTree,
	TreeItem as AriaTreeItem,
	TreeItemContent as AriaTreeItemContent,
	type TreeItemProps,
	type TreeProps,
} from "react-aria-components/Tree";
import { Button } from "./Button";
import { cn } from "./cn";
export function Tree<T extends object>({
	className,
	...props
}: TreeProps<T> & RefAttributes<HTMLDivElement>) {
	return (
		<AriaTree
			{...props}
			className={composeRenderProps(className, (className) =>
				cn("w-full space-y-1 outline-hidden", className),
			)}
		/>
	);
}
export function TreeItem<T extends object>({
	className,
	...props
}: TreeItemProps<T> & RefAttributes<HTMLDivElement>) {
	return (
		<AriaTreeItem
			{...props}
			className={composeRenderProps(className, (className) =>
				cn(
					"ml-[calc((var(--tree-item-level)-1)*20px)] relative flex min-h-9 cursor-default items-center rounded-md border border-transparent bg-surface-raised px-2 text-sm text-fg outline-hidden transition-colors data-[hovered]:bg-header data-[selected]:border-accent/55 data-[selected]:bg-ui-border data-[focus-visible]:outline-2 data-[focus-visible]:outline-accent",
					className,
				),
			)}
		/>
	);
}
export function TreeItemContent({
	children,
	...props
}: ComponentProps<typeof AriaTreeItemContent>) {
	return (
		<AriaTreeItemContent {...props}>
			{composeRenderProps(
				children,
				(children, { hasChildItems, isExpanded }) => (
					<>
						{hasChildItems && (
							<Button
								slot="chevron"
								aria-label={isExpanded ? "Collapse" : "Expand"}
								variant="ghost"
								className="mr-1 size-5 min-h-0 shrink-0 p-0 text-fg"
							>
								{isExpanded ? "▾" : "▸"}
							</Button>
						)}
						<span className="min-w-0 flex-1">{children}</span>
					</>
				),
			)}
		</AriaTreeItemContent>
	);
}
