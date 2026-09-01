import type { CSSProperties, MouseEventHandler, ReactNode } from "react";
import {
	Button as AriaButton,
	type ButtonProps as AriaButtonProps,
} from "react-aria-components/Button";
import { cn } from "./cn";

export type ButtonVariant =
	| "primary"
	| "secondary"
	| "ghost"
	| "danger"
	| "contained"
	| "outlined"
	| "text";
export type ButtonSize = "sm" | "md" | "small" | "medium" | "large";

const variantClasses: Record<ButtonVariant, string> = {
	primary:
		"border-primary bg-primary text-slate-950 data-[hovered]:border-cyan-300 data-[hovered]:bg-cyan-300",
	secondary:
		"border-ui-border bg-surface-raised text-fg data-[hovered]:border-creative data-[hovered]:text-violet-200",
	ghost:
		"border-transparent bg-transparent text-muted data-[hovered]:bg-slate-800 data-[hovered]:text-fg",
	danger:
		"border-red-400/60 bg-red-500/15 text-red-200 data-[hovered]:border-red-300 data-[hovered]:bg-red-500/25",
	contained:
		"border-compat-primary bg-compat-primary text-slate-950 data-[hovered]:border-[#a6d4fa] data-[hovered]:bg-[#a6d4fa]",
	outlined:
		"border-compat-primary/50 bg-transparent text-compat-primary data-[hovered]:border-compat-primary data-[hovered]:bg-compat-primary/8",
	text: "border-transparent bg-transparent text-compat-primary data-[hovered]:bg-compat-primary/8",
};

const sizeClasses: Record<ButtonSize, string> = {
	sm: "min-h-8 px-3 text-xs",
	md: "min-h-9 px-4 text-sm",
	small: "min-h-8 px-3 text-xs",
	medium: "min-h-9 px-4 text-sm",
	large: "min-h-11 px-5 text-base",
};

export interface ButtonProps
	extends Omit<AriaButtonProps, "children" | "className" | "onClick"> {
	children: ReactNode;
	className?: string;
	variant?: ButtonVariant;
	size?: ButtonSize;
	color?: string;
	fullWidth?: boolean;
	disabled?: boolean;
	startIcon?: ReactNode;
	endIcon?: ReactNode;
	onClick?: MouseEventHandler<HTMLButtonElement>;
	style?: CSSProperties;
	sx?: Record<string, unknown>;
	autoFocus?: boolean;
	type?: "button" | "submit" | "reset";
	[key: string]: any;
}

export function Button({
	children,
	className,
	variant = "primary",
	size = "md",
	isPending,
	disabled,
	color,
	fullWidth,
	startIcon,
	endIcon,
	onClick,
	style,
	sx,
	...props
}: ButtonProps) {
	const normalizedVariant =
		variant === "contained" && color === "error" ? "danger" : variant;
	return (
		<AriaButton
			{...props}
			isPending={isPending}
			isDisabled={props.isDisabled ?? disabled}
			onClick={onClick as any}
			type={props.type ?? "button"}
			className={cn(
				"inline-flex cursor-default items-center justify-center gap-2 rounded-md border font-semibold tracking-wide transition-colors outline-hidden data-[disabled]:cursor-not-allowed data-[disabled]:opacity-45 data-[focus-visible]:outline-2 data-[focus-visible]:outline-solid data-[focus-visible]:outline-offset-2 data-[focus-visible]:outline-focus",
				variantClasses[normalizedVariant],
				sizeClasses[size],
				fullWidth && "w-full",
				className,
			)}
			style={{
				...(sx
					? Object.fromEntries(
							Object.entries(sx).filter(([key]) => !key.startsWith("&")),
						)
					: {}),
				...style,
			}}
		>
			{startIcon}
			{isPending && (
				<span
					aria-hidden="true"
					className="size-3.5 animate-spin rounded-full border-2 border-current border-r-transparent"
				/>
			)}
			{children}
			{endIcon}
		</AriaButton>
	);
}
