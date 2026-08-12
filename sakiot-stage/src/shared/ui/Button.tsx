import type { ReactNode } from "react";
import {
	Button as AriaButton,
	type ButtonProps as AriaButtonProps,
} from "react-aria-components/Button";
import { cn } from "./cn";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md";

const variantClasses: Record<ButtonVariant, string> = {
	primary:
		"border-primary bg-primary text-slate-950 data-[hovered]:border-cyan-300 data-[hovered]:bg-cyan-300",
	secondary:
		"border-ui-border bg-surface-raised text-fg data-[hovered]:border-creative data-[hovered]:text-violet-200",
	ghost:
		"border-transparent bg-transparent text-muted data-[hovered]:bg-slate-800 data-[hovered]:text-fg",
	danger:
		"border-red-400/60 bg-red-500/15 text-red-200 data-[hovered]:border-red-300 data-[hovered]:bg-red-500/25",
};

const sizeClasses: Record<ButtonSize, string> = {
	sm: "min-h-8 px-3 text-xs",
	md: "min-h-9 px-4 text-sm",
};

export interface ButtonProps
	extends Omit<AriaButtonProps, "children" | "className"> {
	children: ReactNode;
	className?: string;
	variant?: ButtonVariant;
	size?: ButtonSize;
}

export function Button({
	children,
	className,
	variant = "primary",
	size = "md",
	isPending,
	...props
}: ButtonProps) {
	return (
		<AriaButton
			{...props}
			isPending={isPending}
			className={cn(
				"inline-flex cursor-default items-center justify-center gap-2 rounded-md border font-semibold tracking-wide transition-colors outline-hidden data-[disabled]:cursor-not-allowed data-[disabled]:opacity-45 data-[focus-visible]:outline-2 data-[focus-visible]:outline-solid data-[focus-visible]:outline-offset-2 data-[focus-visible]:outline-focus",
				variantClasses[variant],
				sizeClasses[size],
				className,
			)}
		>
			{isPending && (
				<span
					aria-hidden="true"
					className="size-3.5 animate-spin rounded-full border-2 border-current border-r-transparent"
				/>
			)}
			{children}
		</AriaButton>
	);
}
