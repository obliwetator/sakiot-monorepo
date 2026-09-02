import {
	type ChangeEvent,
	type KeyboardEvent as ReactKeyboardEvent,
	type ReactNode,
	type PointerEvent as ReactPointerEvent,
	useRef,
	useState,
} from "react";
import { cn } from "./cn";
import { omitCompatProps, type SxProps, sxToStyle } from "./theme";

export interface SliderProps {
	value?: number | number[];
	min?: number;
	max?: number;
	step?: number;
	onChange?: (
		event: Event,
		value: number | number[],
		activeThumb: number,
	) => void;
	onChangeCommitted?: (event: Event, value: number | number[]) => void;
	onChangeEnd?: (value: number | number[]) => void;
	valueLabelFormat?: (value: number) => ReactNode;
	getAriaValueText?: (value: number) => string;
	sx?: SxProps;
	className?: string;
	orientation?: "horizontal" | "vertical";
	getAriaLabel?: (index: number) => string;
	"aria-label"?: string;
	disabled?: boolean;
	disableSwap?: boolean;
	[key: string]: any;
}

export function Slider({
	value,
	min = 0,
	max = 100,
	step = 1,
	onChange,
	onChangeCommitted,
	onChangeEnd,
	sx,
	className,
	orientation = "horizontal",
	getAriaLabel,
	getAriaValueText,
	"aria-label": ariaLabel,
	disabled,
	disableSwap = false,
	...props
}: SliderProps) {
	const values: number[] = Array.isArray(value)
		? value
		: [Number(value ?? min)];
	const range = values.length > 1;
	const rangeRef = useRef<HTMLDivElement | null>(null);
	const valuesRef = useRef(values);
	valuesRef.current = values;
	const activeThumbRef = useRef<number | null>(null);
	const [activeThumb, setActiveThumb] = useState<number | null>(null);
	const clamp = (next: number) => Math.min(max, Math.max(min, next));
	const snap = (next: number) => {
		if (!Number.isFinite(next)) return min;
		const steps = Math.round((next - min) / step);
		return clamp(min + steps * step);
	};
	const orderedValue = (index: number, nextValue: number) => {
		const next = [...valuesRef.current];
		next[index] = snap(nextValue);
		if (range && disableSwap) {
			if (index === 0) next[index] = Math.min(next[index], next[1]);
			else next[index] = Math.max(next[index], next[0]);
		}
		if (range && !disableSwap) next.sort((a, b) => a - b);
		return next;
	};
	const emitRangeChange = (index: number, next: number[], event: Event) => {
		valuesRef.current = next;
		onChange?.(event, range ? next : next[0], index);
	};
	const valueFromPointer = (clientX: number) => {
		const bounds = rangeRef.current?.getBoundingClientRect();
		if (!bounds || bounds.width <= 0) return min;
		return snap(min + ((clientX - bounds.left) / bounds.width) * (max - min));
	};
	const finishRangeChange = (event: Event) => {
		const next = valuesRef.current;
		onChangeCommitted?.(event, next);
		onChangeEnd?.(next);
		activeThumbRef.current = null;
		setActiveThumb(null);
	};
	const handleRangePointerDown = (
		index: number,
		event: ReactPointerEvent<HTMLButtonElement>,
	) => {
		if (disabled) return;
		event.preventDefault();
		event.stopPropagation();
		activeThumbRef.current = index;
		setActiveThumb(index);
		event.currentTarget.setPointerCapture(event.pointerId);
		emitRangeChange(
			index,
			orderedValue(index, valueFromPointer(event.clientX)),
			event.nativeEvent,
		);
	};
	const handleRangePointerMove = (
		index: number,
		event: ReactPointerEvent<HTMLButtonElement>,
	) => {
		if (activeThumbRef.current !== index) return;
		emitRangeChange(
			index,
			orderedValue(index, valueFromPointer(event.clientX)),
			event.nativeEvent,
		);
	};
	const handleRangePointerUp = (
		index: number,
		event: ReactPointerEvent<HTMLButtonElement>,
	) => {
		if (activeThumbRef.current !== index) return;
		if (event.currentTarget.hasPointerCapture(event.pointerId)) {
			event.currentTarget.releasePointerCapture(event.pointerId);
		}
		finishRangeChange(event.nativeEvent);
	};
	const handleRangeKeyDown = (
		index: number,
		event: ReactKeyboardEvent<HTMLButtonElement>,
	) => {
		if (disabled) return;
		let nextValue: number | null = null;
		const current = valuesRef.current[index] ?? min;
		if (event.key === "ArrowLeft" || event.key === "ArrowDown") {
			nextValue = current - step;
		} else if (event.key === "ArrowRight" || event.key === "ArrowUp") {
			nextValue = current + step;
		} else if (event.key === "PageDown") {
			nextValue = current - (max - min) / 10;
		} else if (event.key === "PageUp") {
			nextValue = current + (max - min) / 10;
		} else if (event.key === "Home") {
			nextValue = min;
		} else if (event.key === "End") {
			nextValue = max;
		}
		if (nextValue === null) return;
		event.preventDefault();
		const next = orderedValue(index, nextValue);
		emitRangeChange(index, next, event.nativeEvent);
	};
	const handleRangeTrackPointerDown = (
		event: ReactPointerEvent<HTMLDivElement>,
	) => {
		if (disabled || event.target !== event.currentTarget) return;
		const pointerValue = valueFromPointer(event.clientX);
		const current = valuesRef.current;
		const index =
			Math.abs((current[0] ?? min) - pointerValue) <=
			Math.abs((current[1] ?? max) - pointerValue)
				? 0
				: 1;
		emitRangeChange(
			index,
			orderedValue(index, pointerValue),
			event.nativeEvent,
		);
	};
	if (range) {
		const toPercent = (current: number) =>
			((clamp(current) - min) / Math.max(1, max - min)) * 100;
		const lower = toPercent(values[0] ?? min);
		const upper = toPercent(values[1] ?? max);
		return (
			<div
				ref={rangeRef}
				className={cn(
					"relative flex min-h-6 w-full touch-none items-center",
					disabled && "opacity-50",
					orientation === "vertical" && "h-32 w-6",
					className,
				)}
				style={sxToStyle(sx)}
				onPointerDown={handleRangeTrackPointerDown}
			>
				<span
					aria-hidden="true"
					className="pointer-events-none absolute left-0 right-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-slate-700"
				/>
				<span
					aria-hidden="true"
					className="pointer-events-none absolute top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-compat-primary"
					style={{ left: `${lower}%`, width: `${Math.max(0, upper - lower)}%` }}
				/>
				{values.map((current, index) => (
					<button
						key={`slider-thumb-${index}`}
						type="button"
						role="slider"
						aria-label={
							getAriaLabel?.(index) ??
							ariaLabel ??
							(index === 0 ? "Minimum" : "Maximum")
						}
						aria-valuemin={min}
						aria-valuemax={max}
						aria-valuenow={current}
						aria-valuetext={getAriaValueText?.(current)}
						disabled={disabled}
						className={cn(
							"absolute top-1/2 z-10 h-[25px] w-[5px] -translate-x-1/2 -translate-y-1/2 cursor-ew-resize rounded-[1px] border-0 bg-compat-primary p-0 shadow-[0_1px_3px_rgba(2,6,23,0.7)] outline-hidden focus-visible:outline-2 focus-visible:outline-focus",
							activeThumb === index && "z-20",
						)}
						style={{ left: `${toPercent(current)}%` }}
						onPointerDown={(event) => handleRangePointerDown(index, event)}
						onPointerMove={(event) => handleRangePointerMove(index, event)}
						onPointerUp={(event) => handleRangePointerUp(index, event)}
						onPointerCancel={(event) => handleRangePointerUp(index, event)}
						onKeyDown={(event) => handleRangeKeyDown(index, event)}
						onKeyUp={(event) => {
							if (
								event.key === "ArrowLeft" ||
								event.key === "ArrowRight" ||
								event.key === "ArrowUp" ||
								event.key === "ArrowDown" ||
								event.key === "Home" ||
								event.key === "End" ||
								event.key === "PageUp" ||
								event.key === "PageDown"
							) {
								finishRangeChange(event.nativeEvent);
							}
						}}
					/>
				))}
			</div>
		);
	}
	const emit = (index: number, event: ChangeEvent<HTMLInputElement>) => {
		const next = [...values];
		next[index] = Number(event.currentTarget.value);
		if (range) next.sort((a, b) => a - b);
		onChange?.(event.nativeEvent, range ? next : next[0], index);
	};
	const inputs = values.map((current, index) => (
		<input
			key={`slider-input-${index}`}
			{...omitCompatProps(props)}
			type="range"
			min={min}
			max={max}
			step={step}
			value={current}
			disabled={disabled}
			aria-label={
				range
					? (getAriaLabel?.(index) ?? (index === 0 ? "Minimum" : "Maximum"))
					: (getAriaLabel?.(index) ?? ariaLabel)
			}
			aria-valuetext={getAriaValueText?.(current)}
			onChange={(event) => emit(index, event)}
			onMouseUp={(event) => {
				const target = event.currentTarget;
				const next = range ? values : values[0];
				onChangeCommitted?.(event.nativeEvent, range ? next : next);
				onChangeEnd?.(range ? values : values[0]);
				target.blur();
			}}
			className={cn(
				"h-1.5 w-full accent-compat-primary",
				range && "absolute inset-0",
				orientation === "vertical" && "rotate-[-90deg]",
				className,
			)}
		/>
	));
	return (
		<div
			className={cn(
				"relative flex min-h-6 w-full items-center",
				orientation === "vertical" && "h-32 w-6",
			)}
			style={sxToStyle(sx)}
		>
			{inputs}
		</div>
	);
}
