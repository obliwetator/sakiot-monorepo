import {
	createContext,
	type HTMLAttributes,
	type ReactNode,
	useContext,
	useEffect,
	useId,
	useState,
} from "react";
import { cn } from "./cn";
import { omitCompatProps, type SxProps, sxToStyle } from "./theme";

export interface CollapseProps extends HTMLAttributes<HTMLDivElement> {
	in?: boolean;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function Collapse({
	in: isIn,
	children,
	sx,
	className,
	...props
}: CollapseProps) {
	return isIn ? (
		<div
			{...omitCompatProps(props)}
			className={className}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	) : null;
}

export interface TooltipProps {
	title?: ReactNode;
	children: ReactNode;
	[key: string]: any;
}

export function Tooltip({ title, children }: TooltipProps) {
	const [anchor, setAnchor] = useState<{ left: number; top: number } | null>(
		null,
	);
	const tooltipId = useId();
	if (!title) return children;

	const isTextTitle = typeof title === "string";
	const showTooltip = (
		target: EventTarget | null,
		clientX?: number,
		clientY?: number,
	) => {
		if (isTextTitle) return;
		const element = target instanceof HTMLElement ? target : null;
		const bounds = element?.getBoundingClientRect();
		setAnchor({
			left: clientX ?? bounds?.left ?? 0,
			top: clientY ?? bounds?.top ?? 0,
		});
	};

	return (
		<span
			id={isTextTitle ? undefined : tooltipId}
			title={isTextTitle ? title : undefined}
			className="contents"
			onPointerEnter={(event) =>
				showTooltip(event.target, event.clientX, event.clientY)
			}
			onPointerMove={(event) =>
				showTooltip(event.target, event.clientX, event.clientY)
			}
			onPointerLeave={() => setAnchor(null)}
			onFocusCapture={(event) => showTooltip(event.target)}
			onBlurCapture={() => setAnchor(null)}
			aria-describedby={!isTextTitle && anchor ? tooltipId : undefined}
		>
			{children}
			{!isTextTitle && anchor && (
				<span
					role="tooltip"
					className="pointer-events-none fixed z-[80] max-w-72 -translate-x-1/2 -translate-y-full rounded bg-slate-950 px-2 py-1 text-xs leading-4 text-slate-100 shadow-lg ring-1 ring-white/15"
					style={{ left: anchor.left, top: anchor.top - 8 }}
				>
					{title}
				</span>
			)}
		</span>
	);
}

interface MenuContextValue {
	inMenu: boolean;
}
const MenuContext = createContext<MenuContextValue>({ inMenu: false });

export interface MenuProps extends HTMLAttributes<HTMLDivElement> {
	open?: boolean;
	onClose?: (event: unknown) => void;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function Menu({
	open,
	onClose,
	children,
	sx,
	className,
	...props
}: MenuProps) {
	useEffect(() => {
		if (!open) return;
		const handler = (event: KeyboardEvent) =>
			event.key === "Escape" && onClose?.(event);
		document.addEventListener("keydown", handler);
		return () => document.removeEventListener("keydown", handler);
	}, [open, onClose]);
	if (!open) return null;
	return (
		<div
			{...omitCompatProps(props)}
			role="menu"
			className={cn(
				"fixed right-4 top-16 z-50 min-w-40 rounded-md border border-ui-border bg-surface p-1 shadow-2xl",
				className,
			)}
			style={sxToStyle(sx)}
			onMouseDown={(event) => event.stopPropagation()}
		>
			<MenuContext.Provider value={{ inMenu: true }}>
				{children}
			</MenuContext.Provider>
		</div>
	);
}

export interface MenuItemProps extends HTMLAttributes<HTMLElement> {
	value?: unknown;
	onClick?: (event: unknown) => void;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function MenuItem({
	children,
	value,
	onClick,
	sx,
	className,
	...props
}: MenuItemProps) {
	const { inMenu } = useContext(MenuContext);
	if (!inMenu && value !== undefined)
		return (
			<option
				value={value as string | number}
				className="bg-header text-fg"
				style={{
					backgroundColor: "var(--color-header)",
					color: "var(--color-fg)",
				}}
			>
				{children}
			</option>
		);
	if (!inMenu)
		return (
			<div
				{...omitCompatProps(props)}
				className={cn("flex items-center gap-0 px-4 py-1.5", className)}
				style={sxToStyle(sx)}
			>
				{children}
			</div>
		);
	return (
		<button
			type="button"
			role="menuitem"
			{...omitCompatProps(props)}
			onClick={onClick}
			className={cn(
				"block w-full rounded px-3 py-2 text-left text-sm hover:bg-slate-800 focus-visible:outline-2 focus-visible:outline-focus",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</button>
	);
}

export interface PopoverProps extends HTMLAttributes<HTMLDivElement> {
	open?: boolean;
	onClose?: () => void;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function Popover({
	open,
	onClose: _onClose,
	children,
	sx,
	className,
	...props
}: PopoverProps) {
	if (!open) return null;
	return (
		<div
			{...omitCompatProps(props)}
			role="dialog"
			className={cn(
				"fixed left-1/2 top-1/2 z-40 max-w-[calc(100vw-2rem)] -translate-x-1/2 -translate-y-1/2 rounded-md border border-ui-border bg-surface shadow-2xl",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}
