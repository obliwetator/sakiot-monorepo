import type { ComponentPropsWithoutRef, ReactNode } from "react";
import { cn } from "./cn";

export function Page({
	className,
	...props
}: ComponentPropsWithoutRef<"main">) {
	return (
		<main
			className={cn(
				"mx-auto w-full max-w-6xl px-4 py-6 sm:px-6 sm:py-8",
				className,
			)}
			{...props}
		/>
	);
}

export function Panel({
	className,
	...props
}: ComponentPropsWithoutRef<"section">) {
	return (
		<section
			className={cn(
				"rounded-panel border border-ui-border bg-surface p-4 shadow-panel sm:p-5",
				className,
			)}
			{...props}
		/>
	);
}

export function PageTitle({
	className,
	...props
}: ComponentPropsWithoutRef<"h1">) {
	return (
		<h1
			className={cn(
				"text-2xl font-bold tracking-tight text-fg sm:text-3xl",
				className,
			)}
			{...props}
		/>
	);
}

export function SectionTitle({
	className,
	...props
}: ComponentPropsWithoutRef<"h2">) {
	return (
		<h2
			className={cn(
				"text-base font-semibold tracking-tight text-fg sm:text-lg",
				className,
			)}
			{...props}
		/>
	);
}

export interface TextProps extends ComponentPropsWithoutRef<"p"> {
	children: ReactNode;
	tone?: "default" | "muted";
}

export function Text({ className, tone = "default", ...props }: TextProps) {
	return (
		<p
			className={cn(
				"text-sm leading-6",
				tone === "muted" ? "text-muted" : "text-slate-200",
				className,
			)}
			{...props}
		/>
	);
}
