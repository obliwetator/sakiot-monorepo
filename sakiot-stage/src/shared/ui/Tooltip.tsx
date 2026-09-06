import type { ComponentProps } from "react";
import {
	Tooltip as AriaTooltip,
	composeRenderProps,
} from "react-aria-components";
import { cn } from "./cn";

export { Focusable, TooltipTrigger } from "react-aria-components";

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
