import type { ComponentProps, ReactNode } from "react";
import {
	Tooltip as AriaTooltip,
	composeRenderProps,
	TooltipTrigger,
} from "react-aria-components";
import { cn } from "./cn";
import { IconButton, type IconButtonProps } from "./IconButton";

export { Focusable } from "react-aria-components";
export { TooltipTrigger };

export function Tooltip({
	className,
	...props
}: ComponentProps<typeof AriaTooltip>) {
	return (
		<AriaTooltip
			{...props}
			className={composeRenderProps(className, (className) =>
				cn(
					"z-[80] max-w-72 rounded bg-slate-950 px-2 py-1 text-xs leading-4 text-slate-100 shadow-lg ring-1 ring-white/15",
					className,
				),
			)}
		/>
	);
}

/** The delay every tooltip in the app opens after. */
const TOOLTIP_DELAY_MS = 400;

/** Wraps any focusable control in the standard hover/focus tooltip. */
export function WithTooltip({
	tip,
	delay = TOOLTIP_DELAY_MS,
	children,
}: {
	tip: ReactNode;
	delay?: number;
	children: ReactNode;
}) {
	return (
		<TooltipTrigger delay={delay}>
			{children}
			<Tooltip>{tip}</Tooltip>
		</TooltipTrigger>
	);
}

/**
 * An icon button whose accessible name is also its tooltip. Pass `tip`
 * separately when the tooltip should say more than the label.
 */
export function TooltipIconButton({
	label,
	tip,
	icon,
	delay,
	...props
}: Omit<IconButtonProps, "children" | "aria-label"> & {
	label: string;
	tip?: ReactNode;
	icon: ReactNode;
	delay?: number;
}) {
	return (
		<WithTooltip tip={tip ?? label} delay={delay}>
			<IconButton {...props} aria-label={label}>
				{icon}
			</IconButton>
		</WithTooltip>
	);
}
