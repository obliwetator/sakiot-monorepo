import {
	ProgressBar as AriaProgressBar,
	type ProgressBarProps,
} from "react-aria-components/ProgressBar";
import { cn } from "./cn";
export function ProgressBar({
	className,
	...props
}: Omit<ProgressBarProps, "className" | "children"> & { className?: string }) {
	return (
		<AriaProgressBar
			aria-label="Progress"
			{...props}
			className={cn(
				"h-1.5 w-full overflow-hidden rounded-full bg-slate-800",
				className,
			)}
		>
			{({ percentage, isIndeterminate }) => (
				<div
					className={cn(
						"h-full rounded-full bg-accent",
						isIndeterminate && "w-1/3 animate-pulse motion-reduce:animate-none",
					)}
					style={isIndeterminate ? undefined : { width: `${percentage}%` }}
				/>
			)}
		</AriaProgressBar>
	);
}
export function Spinner({
	className,
	...props
}: Omit<ProgressBarProps, "className" | "children"> & { className?: string }) {
	return (
		<AriaProgressBar
			aria-label="Loading"
			{...props}
			isIndeterminate
			className={cn(
				"inline-block size-6 animate-spin rounded-full border-2 border-current border-r-transparent motion-reduce:animate-none",
				className,
			)}
		/>
	);
}
