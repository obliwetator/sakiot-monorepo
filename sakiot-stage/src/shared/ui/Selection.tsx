import type { RefAttributes } from "react";
import {
	Radio as AriaRadio,
	RadioGroup as AriaRadioGroup,
	Switch as AriaSwitch,
	composeRenderProps,
	type RadioGroupProps,
	type RadioProps,
	type SwitchProps,
} from "react-aria-components";
import { cn } from "./cn";

export function Switch({
	children,
	className,
	...props
}: SwitchProps & RefAttributes<HTMLLabelElement>) {
	return (
		<AriaSwitch
			{...props}
			className={composeRenderProps(className, (className) =>
				cn(
					"group inline-flex cursor-default items-center gap-2 text-sm data-[disabled]:opacity-50",
					className,
				),
			)}
		>
			{composeRenderProps(children, (children) => (
				<>
					<span
						aria-hidden="true"
						className="flex h-5 w-9 shrink-0 items-center rounded-full bg-slate-600 p-0.5 transition-colors group-data-[selected]:bg-accent group-data-[focus-visible]:outline-2 group-data-[focus-visible]:outline-offset-2 group-data-[focus-visible]:outline-focus"
					>
						<span className="size-4 rounded-full bg-white transition-transform group-data-[selected]:translate-x-4 motion-reduce:transition-none" />
					</span>
					{children}
				</>
			))}
		</AriaSwitch>
	);
}
export function RadioGroup({
	className,
	...props
}: RadioGroupProps & RefAttributes<HTMLDivElement>) {
	return (
		<AriaRadioGroup
			{...props}
			className={composeRenderProps(className, (className) =>
				cn("flex flex-col gap-2", className),
			)}
		/>
	);
}
export function Radio({
	children,
	className,
	...props
}: RadioProps & RefAttributes<HTMLLabelElement>) {
	return (
		<AriaRadio
			{...props}
			className={composeRenderProps(className, (className) =>
				cn(
					"group inline-flex cursor-default items-center gap-2 text-sm data-[disabled]:opacity-50",
					className,
				),
			)}
		>
			{composeRenderProps(children, (children) => (
				<>
					<span
						aria-hidden="true"
						className="size-4 rounded-full border-2 border-slate-500 group-data-[selected]:border-[5px] group-data-[selected]:border-accent group-data-[focus-visible]:outline-2 group-data-[focus-visible]:outline-offset-2 group-data-[focus-visible]:outline-focus"
					/>
					{children}
				</>
			))}
		</AriaRadio>
	);
}
