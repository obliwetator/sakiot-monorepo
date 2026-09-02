import {
	Children,
	createContext,
	type HTMLAttributes,
	isValidElement,
	type ReactNode,
	type SyntheticEvent,
	useContext,
} from "react";
import { cn } from "./cn";
import { omitCompatProps, type SxProps, sxToStyle } from "./theme";

export interface SimpleTreeViewProps extends HTMLAttributes<HTMLDivElement> {
	expandedItems?: string[];
	selectedItems?: string | null;
	onExpandedItemsChange?: (
		event: SyntheticEvent | null,
		itemIds: string[],
	) => void;
	onSelectedItemsChange?: (
		event: SyntheticEvent | null,
		itemId: string | null,
	) => void;
	onItemClick?: (event: SyntheticEvent | null, itemId: string) => void;
	children?: ReactNode;
	className?: string;
	[key: string]: any;
}

interface TreeContextValue {
	expanded: Set<string>;
	selected: string | null;
	toggle: (id: string) => void;
	select: (id: string) => void;
}
const TreeContext = createContext<TreeContextValue | null>(null);
const TreeLevelContext = createContext({ depth: 0, isLast: true });

function toggleId(ids: string[], id: string) {
	return ids.includes(id) ? ids.filter((item) => item !== id) : [...ids, id];
}

export function SimpleTreeView(props: SimpleTreeViewProps) {
	const expanded = new Set<string>(props.expandedItems ?? []);
	const context: TreeContextValue = {
		expanded,
		selected: props.selectedItems ?? null,
		toggle: (id) =>
			props.onExpandedItemsChange?.(null, toggleId([...expanded], id)),
		select: (id) => {
			props.onSelectedItemsChange?.(null, id);
			props.onItemClick?.(null, id);
		},
	};
	return (
		<TreeContext.Provider value={context}>
			<TreeLevelContext.Provider value={{ depth: 0, isLast: true }}>
				<div
					role="tree"
					{...omitCompatProps(props)}
					className={cn("w-full", props.className)}
				>
					{props.children}
				</div>
			</TreeLevelContext.Provider>
		</TreeContext.Provider>
	);
}

export interface TreeItemProps extends HTMLAttributes<HTMLDivElement> {
	itemId: string;
	label?: ReactNode;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function TreeItem({
	itemId,
	label,
	children,
	className,
	sx,
	...props
}: TreeItemProps) {
	const parent = useContext(TreeContext);
	const { depth, isLast } = useContext(TreeLevelContext);
	const expanded = parent?.expanded.has(itemId) ?? false;
	const selected = parent?.selected === itemId;
	const childItems = Children.toArray(children);
	const hasChildren = childItems.length > 0;
	const handleRowClick = () => {
		if (hasChildren) parent?.toggle(itemId);
		else parent?.select(itemId);
	};
	return (
		<div
			{...omitCompatProps(props)}
			role="treeitem"
			tabIndex={-1}
			aria-expanded={hasChildren ? expanded : undefined}
			aria-selected={selected}
			className={cn("relative", className)}
			style={sxToStyle(sx)}
		>
			{depth > 0 && (
				<>
					<span
						aria-hidden="true"
						className="pointer-events-none absolute -left-2 top-[18px] w-2 border-t-2 border-ui-border"
					/>
					{!isLast && (
						<span
							aria-hidden="true"
							className="pointer-events-none absolute -bottom-[22px] -left-2 top-[18px] border-l-2 border-ui-border"
						/>
					)}
				</>
			)}
			{/* biome-ignore lint/a11y/noStaticElementInteractions: the row handles mouse and keyboard selection */}
			{/* biome-ignore lint/a11y/useKeyWithClickEvents: the row handles mouse and keyboard selection */}
			<div
				className={cn(
					"relative flex min-h-9 w-full cursor-pointer items-center rounded-md border border-transparent bg-surface-raised px-2 text-left text-sm text-fg transition-colors hover:bg-header focus-within:outline-2 focus-within:outline-compat-primary focus-within:outline-offset-1",
					selected &&
						"border-compat-primary/55 bg-ui-border text-fg shadow-panel",
				)}
				onClick={(event) => {
					event.stopPropagation();
					handleRowClick();
				}}
			>
				{hasChildren && (
					<button
						type="button"
						aria-label={expanded ? "Collapse" : "Expand"}
						onClick={(event) => {
							event.stopPropagation();
							parent?.toggle(itemId);
						}}
						className="mr-1 inline-flex size-5 shrink-0 items-center justify-center rounded text-fg hover:bg-ui-border"
					>
						{expanded ? "▾" : "▸"}
					</button>
				)}
				<div className="min-w-0 flex-1 text-left">{label}</div>
			</div>
			{hasChildren && expanded && (
				<div className="relative ml-3 space-y-1 pl-2 pt-1">
					<span
						aria-hidden="true"
						className="pointer-events-none absolute left-0 top-0 h-[22px] border-l-2 border-ui-border"
					/>
					{childItems.map((child, index) => {
						const key =
							isValidElement(child) && child.key != null ? child.key : index;
						return (
							<TreeLevelContext.Provider
								key={key}
								value={{
									depth: depth + 1,
									isLast: index === childItems.length - 1,
								}}
							>
								{child}
							</TreeLevelContext.Provider>
						);
					})}
				</div>
			)}
		</div>
	);
}

export const treeItemClasses = {
	content: "tree-item-content",
	selected: "tree-item-selected",
	iconContainer: "tree-item-icon",
};
