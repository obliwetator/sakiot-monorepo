import { composeRenderProps } from "react-aria-components";
import { Button, type ButtonProps } from "./Button";
import { cn } from "./cn";
export type IconButtonProps = ButtonProps;
export function IconButton({
	className,
	size = "sm",
	variant = "ghost",
	...props
}: IconButtonProps) {
	return (
		<Button
			{...props}
			variant={variant}
			size={size}
			className={composeRenderProps(className, (className) =>
				cn(
					size === "lg"
						? "size-12 p-3"
						: size === "sm"
							? "size-8 p-0"
							: "size-10 p-0",
					className,
				),
			)}
		/>
	);
}
