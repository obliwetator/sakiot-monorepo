import type { ReactNode, RefAttributes } from "react";
import { composeRenderProps } from "react-aria-components";
import {
	Disclosure as AriaDisclosure,
	DisclosurePanel as AriaDisclosurePanel,
	type DisclosurePanelProps,
	type DisclosureProps,
} from "react-aria-components/Disclosure";
import { Heading } from "react-aria-components/Heading";
import { Button, type ButtonProps } from "./Button";
import { cn } from "./cn";

export function Disclosure({
	className,
	...props
}: DisclosureProps & RefAttributes<HTMLDivElement>) {
	return (
		<AriaDisclosure
			{...props}
			className={composeRenderProps(className, (className) =>
				cn(
					"group overflow-hidden rounded-md border border-ui-border",
					className,
				),
			)}
		/>
	);
}
export function DisclosureTrigger({
	children,
	className,
	icon,
	...props
}: ButtonProps & { icon?: ReactNode }) {
	return (
		<Heading>
			<Button
				{...props}
				slot="trigger"
				variant="ghost"
				className={composeRenderProps(className, (className) =>
					cn(
						"w-full justify-between px-4 py-3 text-left text-fg data-[hovered]:bg-slate-800/50",
						className,
					),
				)}
			>
				{composeRenderProps(children, (children) => (
					<>
						<span className="min-w-0 flex-1">{children}</span>
						<span
							aria-hidden="true"
							className="transition-transform group-data-[expanded]:rotate-180 motion-reduce:transition-none"
						>
							{icon ?? "⌄"}
						</span>
					</>
				))}
			</Button>
		</Heading>
	);
}
export function DisclosurePanel({
	className,
	...props
}: DisclosurePanelProps & RefAttributes<HTMLDivElement>) {
	return (
		<AriaDisclosurePanel
			{...props}
			className={composeRenderProps(className, (className) =>
				cn("border-t border-ui-border px-4 py-3", className),
			)}
		/>
	);
}
