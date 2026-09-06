import type { RefAttributes } from "react";
import { composeRenderProps } from "react-aria-components";
import { Link as AriaLink, type LinkProps } from "react-aria-components/Link";
import { cn } from "./cn";
export function Link({
	className,
	...props
}: LinkProps & RefAttributes<HTMLAnchorElement>) {
	return (
		<AriaLink
			{...props}
			className={composeRenderProps(className, (className) =>
				cn(
					"text-accent underline-offset-2 data-[hovered]:underline data-[focus-visible]:outline-2 data-[focus-visible]:outline-focus",
					className,
				),
			)}
		/>
	);
}
