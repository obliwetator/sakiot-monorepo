import type { ElementType, HTMLAttributes } from "react";
import {
	omitCompatProps,
	resolveTag,
	type SxProps,
	spacingValue,
	sxToStyle,
} from "./theme";

export interface BoxProps extends HTMLAttributes<HTMLElement> {
	sx?: SxProps;
	component?: ElementType;
	p?: unknown;
	px?: unknown;
	py?: unknown;
	pt?: unknown;
	pr?: unknown;
	pb?: unknown;
	pl?: unknown;
	m?: unknown;
	mx?: unknown;
	my?: unknown;
	mt?: unknown;
	mr?: unknown;
	mb?: unknown;
	ml?: unknown;
	[key: string]: any;
}

export function Box(props: BoxProps) {
	const {
		sx,
		style,
		component,
		p,
		px,
		py,
		pt,
		pr,
		pb,
		pl,
		m,
		mx,
		my,
		mt,
		mr,
		mb,
		ml,
		...rest
	} = props;
	const Component = resolveTag(component, "div");
	const shorthand = {
		...(p !== undefined ? { padding: spacingValue(p) } : {}),
		...(px !== undefined ? { paddingInline: spacingValue(px) } : {}),
		...(py !== undefined ? { paddingBlock: spacingValue(py) } : {}),
		...(pt !== undefined ? { paddingTop: spacingValue(pt) } : {}),
		...(pr !== undefined ? { paddingRight: spacingValue(pr) } : {}),
		...(pb !== undefined ? { paddingBottom: spacingValue(pb) } : {}),
		...(pl !== undefined ? { paddingLeft: spacingValue(pl) } : {}),
		...(m !== undefined ? { margin: spacingValue(m) } : {}),
		...(mx !== undefined ? { marginInline: spacingValue(mx) } : {}),
		...(my !== undefined ? { marginBlock: spacingValue(my) } : {}),
		...(mt !== undefined ? { marginTop: spacingValue(mt) } : {}),
		...(mr !== undefined ? { marginRight: spacingValue(mr) } : {}),
		...(mb !== undefined ? { marginBottom: spacingValue(mb) } : {}),
		...(ml !== undefined ? { marginLeft: spacingValue(ml) } : {}),
	};
	return (
		<Component
			{...omitCompatProps(rest)}
			style={{ ...shorthand, ...sxToStyle(sx), ...style }}
		/>
	);
}
