import type { ComponentProps } from "react";
import { cn } from "./cn";
export type BadgeTone =
	| "neutral"
	| "accent"
	| "creative"
	| "danger"
	| "warning"
	| "success"
	| "info";
export interface BadgeProps extends ComponentProps<"span"> {
	tone?: BadgeTone;
	appearance?: "solid" | "outline";
	size?: "sm" | "md";
}
const tones: Record<BadgeTone, string> = {
	neutral: "border-white/20 bg-white/10 text-slate-200",
	accent: "border-accent/60 bg-accent/15 text-accent",
	creative: "border-creative/60 bg-creative/15 text-violet-200",
	danger: "border-red-400/60 bg-red-500/20 text-red-200",
	warning: "border-amber-400/60 bg-amber-500/20 text-amber-200",
	success: "border-emerald-400/60 bg-emerald-500/20 text-emerald-200",
	info: "border-sky-400/60 bg-sky-500/20 text-sky-200",
};
export function Badge({
	children,
	className,
	tone = "neutral",
	appearance = "solid",
	size = "md",
	...props
}: BadgeProps) {
	return (
		<span
			{...props}
			className={cn(
				"inline-flex shrink-0 self-center items-center justify-center rounded-full border whitespace-nowrap text-center text-xs leading-none select-none",
				size === "sm" ? "h-6 px-2" : "h-8 px-3",
				tones[tone],
				appearance === "outline" && "bg-transparent",
				className,
			)}
		>
			<span className="truncate">{children}</span>
		</span>
	);
}
export interface AvatarProps extends ComponentProps<"div"> {
	src?: string;
	alt?: string;
}
export function Avatar({
	src,
	alt = "",
	children,
	className,
	...props
}: AvatarProps) {
	return (
		<div
			{...props}
			className={cn(
				"relative flex size-10 shrink-0 items-center justify-center overflow-hidden rounded-full bg-slate-700 text-sm font-semibold text-slate-100",
				className,
			)}
		>
			{src ? (
				<img src={src} alt={alt} className="size-full object-cover" />
			) : (
				children
			)}
		</div>
	);
}
