import type { ReactNode, RefAttributes } from "react";
import {
	Select as AriaSelect,
	type SelectProps as AriaSelectProps,
	Button,
	composeRenderProps,
	Label,
	ListBox,
	ListBoxItem,
	type ListBoxItemProps,
	Popover,
	SelectValue,
} from "react-aria-components";
import { cn } from "./cn";

export interface SelectProps<T extends object>
	extends Omit<AriaSelectProps<T>, "children">,
		RefAttributes<HTMLDivElement> {
	label: string;
	labelPlacement?: "above" | "floating";
	children: ReactNode;
}
export function Select<T extends object>({
	label,
	labelPlacement = "above",
	children,
	className,
	...props
}: SelectProps<T>) {
	return (
		<AriaSelect
			{...props}
			className={composeRenderProps(className, (className) =>
				cn(
					labelPlacement === "floating"
						? "relative min-w-0"
						: "flex min-w-0 flex-col gap-1.5",
					className,
				),
			)}
		>
			<Label
				data-slot="select-label"
				className={cn(
					"text-xs text-slate-200",
					labelPlacement === "floating"
						? "absolute left-3 top-0 z-10 -translate-y-1/2 bg-header px-1 font-normal"
						: "font-semibold",
				)}
			>
				{label}
			</Label>
			<Button
				className={cn(
					"flex w-full min-w-0 items-center justify-between gap-2 rounded-md border border-ui-border bg-header px-3 text-sm text-fg outline-hidden data-[focus-visible]:outline-2 data-[focus-visible]:outline-offset-2 data-[focus-visible]:outline-focus data-[disabled]:opacity-50",
					labelPlacement === "floating" ? "h-14" : "h-9",
				)}
			>
				<SelectValue className="truncate" />
				<span aria-hidden="true">▾</span>
			</Button>
			<Popover className="z-[70] min-w-(--trigger-width) overflow-auto rounded-md border border-ui-border bg-surface p-1 shadow-2xl">
				<ListBox className="outline-hidden">{children}</ListBox>
			</Popover>
		</AriaSelect>
	);
}
export function SelectItem<T extends object>({
	className,
	...props
}: ListBoxItemProps<T> & RefAttributes<HTMLDivElement>) {
	return (
		<ListBoxItem
			{...props}
			className={composeRenderProps(className, (className) =>
				cn(
					"cursor-default rounded px-3 py-2 text-sm text-fg outline-hidden data-[focused]:bg-ui-border data-[selected]:text-accent data-[disabled]:opacity-50",
					className,
				),
			)}
		/>
	);
}
