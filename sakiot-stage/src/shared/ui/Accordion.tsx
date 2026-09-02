import {
	type ButtonHTMLAttributes,
	createContext,
	type HTMLAttributes,
	type ReactNode,
	type SyntheticEvent,
	useContext,
	useState,
} from "react";
import { cn } from "./cn";
import { omitCompatProps, type SxProps, sxToStyle } from "./theme";

interface AccordionContextValue {
	expanded: boolean;
	toggle: () => void;
}
const AccordionContext = createContext<AccordionContextValue | null>(null);

export interface AccordionProps
	extends Omit<HTMLAttributes<HTMLDivElement>, "onChange"> {
	expanded?: boolean;
	defaultExpanded?: boolean;
	onChange?: (event: SyntheticEvent, expanded: boolean) => void;
	disableGutters?: boolean;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function Accordion({
	expanded: controlled,
	defaultExpanded = false,
	onChange,
	children,
	disableGutters: _disableGutters,
	sx,
	className,
	...props
}: AccordionProps) {
	const [internal, setInternal] = useState(defaultExpanded);
	const expanded = controlled ?? internal;
	const toggle = () => {
		const next = !expanded;
		if (controlled === undefined) setInternal(next);
		onChange?.({} as SyntheticEvent, next);
	};
	return (
		<AccordionContext.Provider value={{ expanded, toggle }}>
			<div
				{...omitCompatProps(props)}
				className={cn(
					"overflow-hidden rounded-md border border-ui-border",
					className,
				)}
				style={sxToStyle(sx)}
			>
				{children}
			</div>
		</AccordionContext.Provider>
	);
}

export interface AccordionSummaryProps
	extends ButtonHTMLAttributes<HTMLButtonElement> {
	expandIcon?: ReactNode;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function AccordionSummary({
	children,
	expandIcon,
	sx,
	className,
	...props
}: AccordionSummaryProps) {
	const context = useContext(AccordionContext);
	return (
		<button
			type="button"
			{...omitCompatProps(props)}
			aria-expanded={context?.expanded}
			onClick={context?.toggle}
			className={cn(
				"flex w-full items-center justify-between px-4 py-3 text-left hover:bg-slate-800/50",
				className,
			)}
			style={sxToStyle(sx)}
		>
			<span className="min-w-0 flex-1">{children}</span>
			{expandIcon ?? (
				<span aria-hidden="true" className="text-muted">
					⌄
				</span>
			)}
		</button>
	);
}

export interface AccordionDetailsProps extends HTMLAttributes<HTMLDivElement> {
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function AccordionDetails({
	children,
	sx,
	className,
	...props
}: AccordionDetailsProps) {
	const context = useContext(AccordionContext);
	return context?.expanded ? (
		<div
			{...omitCompatProps(props)}
			className={cn("border-t border-ui-border px-4 py-3", className)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	) : null;
}
