import type { HTMLAttributes, ReactNode } from "react";
import { cn } from "./cn";
import {
	omitCompatProps,
	resolveResponsiveValue,
	type SxProps,
	spacingValue,
	sxToStyle,
} from "./theme";

export interface StackProps extends HTMLAttributes<HTMLDivElement> {
	direction?: unknown;
	spacing?: unknown;
	alignItems?: unknown;
	justifyContent?: unknown;
	flexWrap?: unknown;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function Stack(props: StackProps) {
	const {
		direction = "column",
		spacing = 0,
		alignItems,
		justifyContent,
		flexWrap,
		sx,
		style,
		className,
		children,
		...rest
	} = props;
	const resolvedDirection = resolveResponsiveValue(direction);
	const resolvedSpacing = resolveResponsiveValue(spacing);
	const resolvedAlignItems = resolveResponsiveValue(alignItems);
	const resolvedJustifyContent = resolveResponsiveValue(justifyContent);
	const isRow = resolvedDirection === "row";
	return (
		<div
			{...omitCompatProps(rest)}
			className={cn("flex", isRow ? "flex-row" : "flex-col", className)}
			style={{
				alignItems: resolvedAlignItems as any,
				justifyContent: resolvedJustifyContent as any,
				flexWrap: flexWrap as any,
				gap: spacingValue(resolvedSpacing),
				...sxToStyle(sx),
				...style,
			}}
		>
			{children}
		</div>
	);
}
