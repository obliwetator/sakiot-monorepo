import type { RefAttributes } from "react";
import { composeRenderProps } from "react-aria-components";
import {
	Slider as AriaSlider,
	type SliderProps as AriaSliderProps,
	SliderFill,
	SliderThumb,
	SliderTrack,
} from "react-aria-components/Slider";
import { cn } from "./cn";
export interface SliderProps<T extends number | number[] = number>
	extends Omit<AriaSliderProps<T>, "children">,
		RefAttributes<HTMLDivElement> {
	thumbLabels?: string[];
}
export function Slider<T extends number | number[] = number>({
	className,
	thumbLabels,
	...props
}: SliderProps<T>) {
	return (
		<AriaSlider
			{...props}
			data-slot="slider"
			className={composeRenderProps(className, (className) =>
				cn(
					"w-full min-w-0 data-[disabled]:opacity-50 data-[orientation=vertical]:h-32 data-[orientation=vertical]:w-6",
					className,
				),
			)}
		>
			<SliderTrack
				data-slot="slider-track"
				className="group relative flex h-6 w-full touch-none items-center data-[orientation=vertical]:h-full data-[orientation=vertical]:w-6"
			>
				{({ state, orientation }) => (
					<>
						<div className="pointer-events-none absolute h-1.5 w-full rounded-full bg-slate-700 group-data-[orientation=vertical]:h-full group-data-[orientation=vertical]:w-1.5" />
						<SliderFill className="pointer-events-none absolute h-1.5 rounded-full bg-accent group-data-[orientation=vertical]:w-1.5" />
						{state.values.map((_, index) => (
							<SliderThumb
								data-slot="slider-thumb"
								// Thumb identity is its fixed position in the range, not its label.
								// biome-ignore lint/suspicious/noArrayIndexKey: range endpoints never reorder
								key={index}
								index={index}
								aria-label={thumbLabels?.[index]}
								className={cn(
									"z-10 border-0 bg-accent shadow-sm outline-hidden data-[focus-visible]:outline-2 data-[focus-visible]:outline-offset-2 data-[focus-visible]:outline-focus data-[dragging]:z-20",
									state.values.length > 1
										? "h-[25px] w-[5px] rounded-[1px]"
										: "size-3 rounded-full",
									orientation === "horizontal" ? "top-1/2" : "left-1/2",
								)}
							/>
						))}
					</>
				)}
			</SliderTrack>
		</AriaSlider>
	);
}
