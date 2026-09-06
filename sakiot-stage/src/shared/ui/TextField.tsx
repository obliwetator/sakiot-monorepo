import type {
	ComponentProps,
	CSSProperties,
	ReactNode,
	RefAttributes,
} from "react";
import {
	TextArea as AriaTextArea,
	TextField as AriaTextField,
	type TextFieldProps as AriaTextFieldProps,
	composeRenderProps,
	FieldError,
	Input,
	Label,
	Text,
} from "react-aria-components";
import { cn } from "./cn";

export interface TextFieldProps
	extends Omit<AriaTextFieldProps, "children" | "spellCheck">,
		RefAttributes<HTMLDivElement> {
	label?: ReactNode;
	description?: ReactNode;
	error?: string;
	placeholder?: string;
	min?: number;
	max?: number;
	step?: number;
	inputClassName?: string;
	inputStyle?: CSSProperties;
	leadingIcon?: ReactNode;
	spellCheck?: boolean;
	inputMode?: ComponentProps<"input">["inputMode"];
	title?: string;
}
const controlClasses =
	"h-9 min-w-0 w-full rounded-md border border-ui-border bg-slate-950/65 px-3 pb-px text-sm leading-6 text-fg outline-hidden transition placeholder:text-slate-600 max-[899px]:pb-0 data-[hovered]:border-slate-500 data-[focus-visible]:border-primary data-[focus-visible]:outline-2 data-[focus-visible]:outline-solid data-[focus-visible]:outline-offset-2 data-[focus-visible]:outline-focus data-[invalid]:border-danger data-[disabled]:cursor-not-allowed data-[disabled]:opacity-50";
function Field({
	label,
	description,
	error,
	className,
	children,
	spellCheck,
	title,
	...props
}: TextFieldProps & { children: ReactNode }) {
	return (
		<AriaTextField
			{...props}
			isInvalid={props.isInvalid || Boolean(error)}
			className={composeRenderProps(className, (className) =>
				cn("flex min-w-0 flex-col gap-1.5", className),
			)}
		>
			{label && (
				<Label className="text-xs font-semibold tracking-wide text-slate-200">
					{label}
				</Label>
			)}
			{children}
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
export function TextField({
	min,
	max,
	step,
	placeholder,
	inputClassName,
	inputStyle,
	leadingIcon,
	spellCheck,
	inputMode,
	title,
	...props
}: TextFieldProps) {
	const input = (
		<Input
			title={title}
			min={min}
			max={max}
			step={step}
			placeholder={placeholder}
			spellCheck={spellCheck}
			inputMode={inputMode}
			style={inputStyle}
			className={cn(controlClasses, leadingIcon && "pl-9", inputClassName)}
		/>
	);
	return (
		<Field {...props}>
			{leadingIcon ? (
				<div className="relative">
					<span
						aria-hidden="true"
						className="pointer-events-none absolute inset-y-0 left-3 flex items-center text-muted"
					>
						{leadingIcon}
					</span>
					{input}
				</div>
			) : (
				input
			)}
		</Field>
	);
}
export function TextArea({
	rows = 3,
	placeholder,
	inputClassName,
	inputStyle,
	spellCheck,
	...props
}: TextFieldProps & { rows?: number }) {
	return (
		<Field {...props}>
			<AriaTextArea
				rows={rows}
				placeholder={placeholder}
				spellCheck={spellCheck}
				style={inputStyle}
				className={cn(controlClasses, "h-auto py-2", inputClassName)}
			/>
		</Field>
	);
}
