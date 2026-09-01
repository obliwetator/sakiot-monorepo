import type { FocusEventHandler, HTMLInputTypeAttribute } from "react";
import {
	TextField as AriaTextField,
	FieldError,
	Input,
	Label,
	Text,
} from "react-aria-components/TextField";
import { cn } from "./cn";

export interface TextFieldProps {
	value: string;
	onChange: (value: string) => void;
	label: string;
	description?: string;
	error?: string;
	name?: string;
	type?: HTMLInputTypeAttribute;
	min?: number;
	max?: number;
	step?: number;
	isDisabled?: boolean;
	isRequired?: boolean;
	autoComplete?: string;
	onFocus?: FocusEventHandler<HTMLElement>;
	className?: string;
}

export function TextField({
	value,
	onChange,
	label,
	description,
	error,
	type = "text",
	min,
	max,
	step,
	className,
	...props
}: TextFieldProps) {
	return (
		<AriaTextField
			{...props}
			value={value}
			onChange={onChange}
			isInvalid={Boolean(error)}
			className={cn("flex min-w-0 flex-col gap-1.5", className)}
		>
			<Label className="text-xs font-semibold tracking-wide text-slate-200">
				{label}
			</Label>
			<Input
				type={type}
				min={min}
				max={max}
				step={step}
				className="h-9 min-w-0 rounded-md border border-ui-border bg-slate-950/65 px-3 pb-px text-sm leading-6 text-fg outline-hidden transition placeholder:text-slate-600 max-[899px]:pb-0 data-[hovered]:border-slate-500 data-[focus-visible]:border-primary data-[focus-visible]:outline-2 data-[focus-visible]:outline-solid data-[focus-visible]:outline-offset-2 data-[focus-visible]:outline-focus data-[invalid]:border-danger data-[disabled]:cursor-not-allowed data-[disabled]:opacity-50"
			/>
			{description && (
				<Text slot="description" className="text-xs leading-5 text-muted">
					{description}
				</Text>
			)}
			{error && (
				<FieldError className="text-xs font-medium text-red-300">
					{error}
				</FieldError>
			)}
		</AriaTextField>
	);
}
