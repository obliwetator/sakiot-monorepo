import type { Ref } from "react";
import { composeRenderProps } from "react-aria-components";
import {
	Button as AriaButton,
	type ButtonProps as AriaButtonProps,
} from "react-aria-components/Button";
import { cn } from "./cn";

export type ButtonVariant =
	| "primary"
	| "secondary"
	| "outline"
	| "ghost"
	| "danger";
export type ButtonSize = "sm" | "md" | "lg";
const variants: Record<ButtonVariant, string> = {
	primary:
		"border-accent bg-accent text-slate-950 data-[hovered]:border-[#a6d4fa] data-[hovered]:bg-[#a6d4fa]",
	secondary:
		"border-ui-border bg-surface-raised text-fg data-[hovered]:border-creative data-[hovered]:text-violet-200",
	outline:
		"border-accent/50 bg-transparent text-accent data-[hovered]:border-accent data-[hovered]:bg-accent/8",
	ghost:
		"border-transparent bg-transparent text-accent data-[hovered]:bg-accent/8",
	danger:
		"border-red-400/60 bg-red-500/15 text-red-200 data-[hovered]:border-red-300 data-[hovered]:bg-red-500/25",
};
const sizes: Record<ButtonSize, string> = {
	sm: "min-h-8 px-3 text-xs",
	md: "min-h-9 px-4 text-sm",
	lg: "min-h-11 px-5 text-base",
};
export interface ButtonProps extends AriaButtonProps {
	ref?: Ref<HTMLButtonElement>;
	variant?: ButtonVariant;
	size?: ButtonSize;
}
export function Button({
	children,
	className,
	variant = "primary",
	size = "md",
	...props
}: ButtonProps) {
	return (
		<AriaButton
			{...props}
			className={composeRenderProps(className, (className) =>
				cn(
					"inline-flex cursor-default items-center justify-center gap-2 rounded-md border font-semibold tracking-wide transition-colors outline-hidden data-[disabled]:cursor-not-allowed data-[disabled]:opacity-45 data-[focus-visible]:outline-2 data-[focus-visible]:outline-solid data-[focus-visible]:outline-offset-2 data-[focus-visible]:outline-focus",
					variants[variant],
					sizes[size],
					className,
				),
			)}
		>
			{composeRenderProps(children, (children, { isPending }) => (
				<>
					{isPending && (
						<span
							aria-hidden="true"
							className="size-3.5 animate-spin rounded-full border-2 border-current border-r-transparent motion-reduce:animate-none"
						/>
					)}
					{children}
				</>
			))}
		</AriaButton>
	);
}
