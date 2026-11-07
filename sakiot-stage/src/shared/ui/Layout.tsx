import type { ComponentPropsWithoutRef, ReactNode } from "react";
import { cn } from "./cn";

const PAGE_COLUMN = "mx-auto w-full max-w-6xl px-4 sm:px-6";

export interface PageProps
	extends Omit<ComponentPropsWithoutRef<"main">, "title"> {
	title: string;
}

/**
 * Fills the space under the navbar, which never scrolls. The title stays put;
 * everything below it scrolls in its own window.
 */
export function Page({ title, className, children, ...props }: PageProps) {
	return (
		<main className="flex h-full min-h-0 flex-col" {...props}>
			<div className={cn(PAGE_COLUMN, "shrink-0 pt-6 pb-1 sm:pt-8")}>
				<PageTitle>{title}</PageTitle>
			</div>
			<ScrollRegion fill aria-label={title}>
				<div className={cn(PAGE_COLUMN, "pb-6 sm:pb-8", className)}>
					{children}
				</div>
			</ScrollRegion>
		</main>
	);
}

export interface ScrollRegionProps extends ComponentPropsWithoutRef<"section"> {
	"aria-label": string;
	/** Take the space left in a flex column, instead of a list's usual cap. */
	fill?: boolean;
}

/**
 * A window of its own for anything that can outgrow the space it is given.
 * Relative because a scroller must contain the absolutely positioned hidden
 * inputs of radios and switches: otherwise they sit in the document, stretch
 * it, and focusing one scrolls the whole app out of place.
 */
export function ScrollRegion({ fill, className, ...props }: ScrollRegionProps) {
	return (
		<section
			// biome-ignore lint/a11y/noNoninteractiveTabindex: a scrolling region must be focusable to scroll by keyboard.
			tabIndex={0}
			className={cn(
				"relative overflow-auto focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-focus",
				fill ? "min-h-0 flex-1" : "max-h-[60dvh]",
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
