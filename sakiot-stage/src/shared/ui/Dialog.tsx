import {
	createContext,
	type HTMLAttributes,
	type ReactNode,
	useContext,
	useEffect,
	useId,
} from "react";
import { cn } from "./cn";
import { omitCompatProps, type SxProps, sxToStyle } from "./theme";

interface DialogContextValue {
	titleId: string;
}
const DialogContext = createContext<DialogContextValue | null>(null);

export interface DialogProps extends HTMLAttributes<HTMLDivElement> {
	open: boolean;
	onClose?: (event: unknown, reason?: string) => void;
	fullWidth?: boolean;
	maxWidth?: "xs" | "sm" | "md" | "lg" | "xl" | false | string;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function Dialog({
	open,
	onClose,
	children,
	fullWidth,
	maxWidth,
	sx,
	className,
	...props
}: DialogProps) {
	const titleId = useId();
	useEffect(() => {
		if (!open) return;
		const handleKeyDown = (event: KeyboardEvent) => {
			if (event.key === "Escape") onClose?.({}, "escapeKeyDown");
		};
		document.addEventListener("keydown", handleKeyDown);
		return () => document.removeEventListener("keydown", handleKeyDown);
	}, [open, onClose]);
	if (!open) return null;
	return (
		// biome-ignore lint/a11y/noStaticElementInteractions: the backdrop dismisses the controlled dialog.
		<div
			{...omitCompatProps(props)}
			className="fixed inset-0 z-50 flex items-center justify-center p-4"
			role="presentation"
			onMouseDown={(event) => {
				if (event.target === event.currentTarget)
					onClose?.({}, "backdropClick");
			}}
		>
			<div
				role="dialog"
				aria-modal="true"
				aria-labelledby={titleId}
				className={cn(
					"max-h-[calc(100vh-2rem)] w-full max-w-lg overflow-auto rounded-lg border border-ui-border bg-surface shadow-2xl",
					fullWidth && "max-w-2xl",
					maxWidth === "sm" && "max-w-md",
					className,
				)}
				style={sxToStyle(sx)}
			>
				<DialogContext.Provider value={{ titleId }}>
					{children}
				</DialogContext.Provider>
			</div>
		</div>
	);
}

export function DialogTitle({
	children,
	id,
	sx,
	className,
	...props
}: HTMLAttributes<HTMLHeadingElement> & { sx?: SxProps; [key: string]: any }) {
	const context = useContext(DialogContext);
	return (
		<h2
			{...omitCompatProps(props)}
			id={id ?? context?.titleId}
			className={cn(
				"border-b border-ui-border px-5 py-4 text-lg font-semibold",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</h2>
	);
}

export function DialogContent({
	children,
	dividers,
	sx,
	className,
	...props
}: HTMLAttributes<HTMLDivElement> & {
	dividers?: boolean;
	sx?: SxProps;
	[key: string]: any;
}) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(
				"space-y-3 px-5 py-4",
				dividers && "border-y border-ui-border",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}

export function DialogContentText({
	children,
	sx,
	className,
	...props
}: HTMLAttributes<HTMLParagraphElement> & {
	sx?: SxProps;
	[key: string]: any;
}) {
	return (
		<p
			{...omitCompatProps(props)}
			className={cn("text-sm leading-6 text-slate-200", className)}
			style={sxToStyle(sx)}
		>
			{children}
		</p>
	);
}

export function DialogActions({
	children,
	sx,
	className,
	...props
}: HTMLAttributes<HTMLDivElement> & { sx?: SxProps; [key: string]: any }) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(
				"flex justify-end gap-2 border-t border-ui-border px-5 py-3",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}

export interface DrawerProps extends HTMLAttributes<HTMLDivElement> {
	open?: boolean;
	onClose?: (event: unknown, reason?: string) => void;
	anchor?: "left" | "right" | "top" | "bottom";
	sx?: SxProps;
	children?: ReactNode;
	ModalProps?: { keepMounted?: boolean };
	[key: string]: any;
}

export function Drawer({
	open,
	onClose,
	anchor = "left",
	children,
	sx,
	className,
	...props
}: DrawerProps) {
	const keepMounted = Boolean(props.ModalProps?.keepMounted);
	useEffect(() => {
		if (!open) return;
		const handler = (event: KeyboardEvent) =>
			event.key === "Escape" && onClose?.({}, "escapeKeyDown");
		document.addEventListener("keydown", handler);
		return () => document.removeEventListener("keydown", handler);
	}, [open, onClose]);
	if (!open && !keepMounted) return null;
	const edge = anchor === "right" ? "right-0" : "left-0";
	return (
		// biome-ignore lint/a11y/noStaticElementInteractions: the backdrop dismisses the controlled drawer.
		<div
			aria-hidden={!open || undefined}
			className={cn(
				"fixed inset-0 z-40",
				!open && "pointer-events-none invisible",
			)}
			role="presentation"
			onMouseDown={(event) => {
				if (event.target === event.currentTarget)
					onClose?.({}, "backdropClick");
			}}
		>
			<div
				{...omitCompatProps(props)}
				className={cn(
					"absolute bottom-0 top-0 w-80 max-w-[90vw] overflow-auto border-ui-border bg-surface shadow-2xl",
					edge,
					anchor === "left" ? "border-r" : "border-l",
					className,
				)}
				style={sxToStyle(sx)}
			>
				{children}
			</div>
		</div>
	);
}
