import type { ReactNode } from "react";
import type { ButtonProps } from "./Button";
import { Button } from "./Button";
import { cn } from "./cn";

export interface IconButtonProps
	extends Omit<ButtonProps, "aria-label" | "children"> {
	label: string;
	children: ReactNode;
}

export function IconButton({
	label,
	children,
	className,
	size = "sm",
	...props
}: IconButtonProps) {
	return (
		<Button
			{...props}
			aria-label={label}
			size={size}
			className={cn(size === "sm" ? "size-8 p-0" : "size-9 p-0", className)}
		>
			{children}
		</Button>
	);
}
