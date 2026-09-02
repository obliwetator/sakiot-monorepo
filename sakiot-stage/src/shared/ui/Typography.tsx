import type {
	CSSProperties,
	ElementType,
	HTMLAttributes,
	ReactNode,
} from "react";
import { cn } from "./cn";
import {
	omitCompatProps,
	resolveTag,
	type SxProps,
	sxToStyle,
	themeValue,
} from "./theme";

const typographyTags: Record<string, ElementType> = {
	h1: "h1",
	h2: "h2",
	h3: "h3",
	h4: "h4",
	h5: "h5",
	h6: "h6",
	subtitle1: "h6",
	subtitle2: "h6",
	body1: "p",
	body2: "p",
	caption: "span",
};

export interface TypographyProps extends HTMLAttributes<HTMLElement> {
	variant?: string;
	color?: string;
	fontSize?: CSSProperties["fontSize"];
	fontWeight?: CSSProperties["fontWeight"];
	gutterBottom?: boolean;
	noWrap?: boolean;
	textAlign?: CSSProperties["textAlign"];
	display?: CSSProperties["display"];
	sx?: SxProps;
	component?: ElementType;
	children?: ReactNode;
	[key: string]: any;
}

export function Typography(props: TypographyProps) {
	const {
		variant = "body1",
		color,
		fontSize,
		fontWeight,
		gutterBottom,
		noWrap,
		textAlign,
		display,
		sx,
		style,
		className,
		component,
		children,
		...rest
	} = props;
	const Component = resolveTag(component, typographyTags[variant] ?? "p");
	const defaultSize =
		variant === "h5"
			? "1.5rem"
			: variant === "h6"
				? "1.25rem"
				: variant === "subtitle1"
					? "1rem"
					: variant === "caption"
						? "0.75rem"
						: variant === "body2"
							? "0.875rem"
							: undefined;
	return (
		<Component
			{...omitCompatProps(rest)}
			className={cn(
				"leading-6",
				variant.startsWith("h") && "font-semibold tracking-tight",
				variant === "h6" && "font-medium tracking-[0.001em] leading-[1.6]",
				variant === "caption" && "leading-5",
				noWrap && "overflow-hidden text-ellipsis whitespace-nowrap",
				gutterBottom && "mb-2",
				className,
			)}
			style={{
				fontSize: fontSize ?? defaultSize,
				fontWeight,
				color: themeValue(color) as string | undefined,
				textAlign,
				display,
				...sxToStyle(sx),
				...style,
			}}
		>
			{children}
		</Component>
	);
}
