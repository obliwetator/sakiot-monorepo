import type { AnchorHTMLAttributes, ElementType, ReactNode } from "react";
import { cn } from "./cn";
import { omitCompatProps, resolveTag, type SxProps, sxToStyle } from "./theme";

export interface LinkProps extends AnchorHTMLAttributes<HTMLAnchorElement> {
	component?: ElementType;
	to?: string;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function Link({
	children,
	component,
	sx,
	className,
	...props
}: LinkProps) {
	const Component = resolveTag(component, "a");
	return (
		<Component
			{...omitCompatProps(props)}
			className={cn(
				"text-cyan-300 underline-offset-2 hover:underline",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</Component>
	);
}
