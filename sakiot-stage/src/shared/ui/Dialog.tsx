import type { ReactNode, RefAttributes } from "react";
import {
	Modal as AriaModal,
	Dialog,
	Heading,
	ModalOverlay,
	type ModalOverlayProps,
} from "react-aria-components";
import { cn } from "./cn";

export interface ModalProps
	extends Omit<ModalOverlayProps, "children" | "className">,
		RefAttributes<HTMLDivElement> {
	children: ReactNode;
	className?: string;
	"aria-label"?: string;
}
export function Modal({
	children,
	className,
	"aria-label": label,
	...props
}: ModalProps) {
	return (
		<ModalOverlay
			isDismissable
			{...props}
			className="fixed inset-0 z-50 flex items-center justify-center bg-black/45 p-4"
		>
			<AriaModal
				className={cn(
					"max-h-[calc(100dvh-2rem)] w-full max-w-lg overflow-auto rounded-lg border border-ui-border bg-surface text-fg shadow-2xl",
					className,
				)}
			>
				<Dialog aria-label={label} className="outline-hidden">
					{children}
				</Dialog>
			</AriaModal>
		</ModalOverlay>
	);
}
export function DialogHeading({
	className,
	...props
}: React.ComponentProps<typeof Heading>) {
	return (
		<Heading
			{...props}
			slot="title"
			className={cn(
				"border-b border-ui-border px-5 py-4 text-lg font-semibold",
				className,
			)}
		/>
	);
}
export function Drawer({
	children,
	className,
	side = "left",
	"aria-label": label = "Browse",
	...props
}: ModalProps & { side?: "left" | "right" }) {
	return (
		<ModalOverlay
			isDismissable
			{...props}
			className={cn(
				"fixed inset-0 z-50 bg-black/45",
				props.isExiting && !props.isOpen && "invisible pointer-events-none",
			)}
		>
			<AriaModal
				className={cn(
					"absolute inset-y-0 max-w-[85vw] overflow-auto border-ui-border bg-surface text-fg shadow-2xl",
					side === "right" ? "right-0 border-l" : "left-0 border-r",
					className,
				)}
			>
				<Dialog
					aria-label={label}
					className={cn(
						"min-h-full outline-hidden",
						props.isExiting && !props.isOpen && "opacity-0",
					)}
				>
					{children}
				</Dialog>
			</AriaModal>
		</ModalOverlay>
	);
}
