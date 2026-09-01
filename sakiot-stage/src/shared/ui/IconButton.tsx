import type { ReactNode } from "react";
import type { ButtonProps } from "./Button";
import { Button } from "./Button";
import { cn } from "./cn";

export interface IconButtonProps
	extends Omit<ButtonProps, "aria-label" | "children"> {
	label?: string;
	"aria-label"?: string;
	children: ReactNode;
}

export function IconButton({
	label,
	"aria-label": ariaLabel,
	children,
	className,
	size = "sm",
	variant = "ghost",
	...props
}: IconButtonProps) {
	const buttonSize = size === "small" ? "sm" : size === "large" ? "md" : size;
	return (
		<Button
			{...props}
			variant={variant}
			aria-label={label ?? ariaLabel ?? "Icon button"}
			size={buttonSize}
			className={cn(
				size === "large"
					? "size-12 p-3"
					: buttonSize === "sm"
						? "size-8 p-0"
						: "size-10 p-0",
				className,
			)}
		>
			{children}
		</Button>
	);
}
