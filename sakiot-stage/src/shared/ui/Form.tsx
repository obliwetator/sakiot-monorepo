import {
	type ChangeEvent,
	type ChangeEventHandler,
	type CSSProperties,
	createContext,
	type HTMLAttributes,
	type ReactNode,
	useContext,
	useId,
} from "react";
import { cn } from "./cn";
import { omitCompatProps, type SxProps, sxToStyle } from "./theme";

export interface FormControlProps extends HTMLAttributes<HTMLDivElement> {
	fullWidth?: boolean;
	size?: "small" | "medium";
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function FormControl({
	children,
	fullWidth,
	size,
	sx,
	className,
	...props
}: FormControlProps) {
	return (
		<div
			{...omitCompatProps(props)}
			className={cn(
				"relative flex min-w-0",
				fullWidth && "w-full",
				size === "small" && "max-h-9",
				className,
			)}
			style={sxToStyle(sx)}
		>
			{children}
		</div>
	);
}

export interface InputLabelProps extends HTMLAttributes<HTMLSpanElement> {
	htmlFor?: string;
	id?: string;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function InputLabel({
	children,
	htmlFor: _htmlFor,
	id,
	sx,
	className,
	...props
}: InputLabelProps) {
	return (
		<span
			{...omitCompatProps(props)}
			id={id}
			className={cn(
				"absolute left-0 top-0 z-10 bg-surface text-base font-normal leading-[1.4375] text-white/70",
				className,
			)}
			style={{
				transform: "translate(14px, -9px) scale(0.75)",
				transformOrigin: "top left",
				...sxToStyle(sx),
			}}
		>
			{children}
		</span>
	);
}

export interface InputAdornmentProps extends HTMLAttributes<HTMLSpanElement> {
	position?: "start" | "end";
	children?: ReactNode;
	[key: string]: any;
}

export function InputAdornment({
	children,
	position: _position,
}: InputAdornmentProps) {
	return (
		<span className="inline-flex shrink-0 items-center px-2 text-muted">
			{children}
		</span>
	);
}

export interface FormControlLabelProps
	extends HTMLAttributes<HTMLLabelElement> {
	control: ReactNode;
	label: ReactNode;
	value?: unknown;
	sx?: SxProps;
	[key: string]: any;
}

export function FormControlLabel({
	control,
	label,
	value: _value,
	sx,
	className,
	...props
}: FormControlLabelProps) {
	return (
		// biome-ignore lint/a11y/noLabelWithoutControl: the supplied control is rendered inside this label.
		<label
			{...omitCompatProps(props)}
			className={cn("inline-flex items-center gap-2", className)}
			style={sxToStyle(sx)}
		>
			{control}
			{label}
		</label>
	);
}

interface SelectContextValue {
	inSelect: boolean;
}
const SelectContext = createContext<SelectContextValue>({ inSelect: false });

export interface SelectChangeEvent<T = unknown>
	extends ChangeEvent<HTMLSelectElement> {
	target: EventTarget & { value: T } & HTMLSelectElement;
}

export interface SelectProps extends HTMLAttributes<HTMLSelectElement> {
	value?: string | number;
	onChange?: (event: SelectChangeEvent) => void;
	label?: string;
	labelId?: string;
	size?: "small" | "medium";
	sx?: SxProps;
	[key: string]: any;
}

export function Select({
	value,
	onChange,
	children,
	label,
	labelId,
	size,
	sx,
	className,
	...props
}: SelectProps) {
	return (
		<SelectContext.Provider value={{ inSelect: true }}>
			<div className="relative w-full">
				<select
					{...omitCompatProps(props)}
					value={value}
					onChange={onChange}
					aria-label={label}
					aria-labelledby={labelId}
					className={cn(
						size === "small" ? "h-9 text-sm" : "h-14 text-base",
						"w-full appearance-none rounded border border-ui-border bg-header pl-[13px] pr-6 text-fg outline-hidden focus:border-compat-primary focus-visible:outline-2 focus-visible:outline-focus",
						className,
					)}
					style={{
						...sxToStyle(sx),
						colorScheme: "dark",
						backgroundImage:
							"url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='5' shape-rendering='crispEdges'%3E%3Cpath fill='%23939393' d='M0 0h10v1H0z M1 1h8v1H1z M2 2h6v1H2z M3 3h4v1H3z M4 4h2v1H4z'/%3E%3Cpath fill='%23fff' d='M1 0h8v1H1z M2 1h6v1H2z M3 2h4v1H3z M4 3h2v1H4z'/%3E%3C/svg%3E\")",
						backgroundPosition: "right 13px center",
						backgroundRepeat: "no-repeat",
						backgroundSize: "10px 5px",
					}}
				>
					{children}
				</select>
			</div>
		</SelectContext.Provider>
	);
}

interface RadioContextValue {
	value: string;
	onChange?: (event: ChangeEvent<HTMLInputElement>) => void;
	name: string;
}
const RadioContext = createContext<RadioContextValue | null>(null);

export interface RadioGroupProps extends HTMLAttributes<HTMLDivElement> {
	value?: string;
	onChange?: ChangeEventHandler<HTMLInputElement>;
	name?: string;
	sx?: SxProps;
	children?: ReactNode;
	[key: string]: any;
}

export function RadioGroup({
	value,
	onChange,
	children,
	sx,
	className,
	name,
	...props
}: RadioGroupProps) {
	const generated = useId();
	return (
		<RadioContext.Provider
			value={{ value: value ?? "", onChange, name: name ?? generated }}
		>
			<div
				{...omitCompatProps(props)}
				role="radiogroup"
				className={cn("flex flex-col gap-2", className)}
				style={sxToStyle(sx)}
			>
				{children}
			</div>
		</RadioContext.Provider>
	);
}

export interface RadioProps extends HTMLAttributes<HTMLInputElement> {
	value?: string;
	checked?: boolean;
	disabled?: boolean;
	size?: string;
	sx?: SxProps;
	inputProps?: Record<string, unknown>;
	[key: string]: any;
}

export function Radio({
	value,
	checked,
	onChange,
	disabled,
	size: _size,
	sx,
	className,
	inputProps,
	...props
}: RadioProps) {
	const context = useContext(RadioContext);
	return (
		<input
			{...omitCompatProps(props)}
			{...inputProps}
			type="radio"
			name={context?.name}
			value={value}
			checked={checked ?? context?.value === value}
			onChange={onChange ?? context?.onChange}
			disabled={disabled}
			className={cn("size-4 accent-compat-primary", className)}
			style={sxToStyle(sx)}
		/>
	);
}

export interface SwitchProps {
	checked?: boolean;
	onChange?: (event: ChangeEvent<HTMLInputElement>, checked: boolean) => void;
	disabled?: boolean;
	sx?: SxProps;
	className?: string;
	inputProps?: Record<string, unknown>;
	[key: string]: any;
}

export function Switch({
	checked,
	onChange,
	disabled,
	sx,
	className,
	inputProps,
	...props
}: SwitchProps) {
	const handleChange = (event: ChangeEvent<HTMLInputElement>) =>
		onChange?.(event, event.currentTarget.checked);
	return (
		<label
			className={cn(
				"relative inline-flex h-5 w-9 shrink-0 items-center",
				disabled && "opacity-50",
				className,
			)}
			style={sxToStyle(sx)}
		>
			<input
				{...omitCompatProps(props)}
				{...inputProps}
				type="checkbox"
				role="switch"
				aria-checked={checked}
				checked={checked}
				onChange={handleChange}
				disabled={disabled}
				className="peer sr-only"
			/>
			<span
				aria-hidden="true"
				className="absolute inset-0 rounded-full bg-slate-700 transition-colors peer-checked:bg-compat-primary peer-focus-visible:outline-2 peer-focus-visible:outline-focus"
			/>
			<span
				aria-hidden="true"
				className="absolute left-0.5 size-4 rounded-full bg-white transition-transform peer-checked:translate-x-4"
			/>
		</label>
	);
}

export interface LegacyTextFieldProps extends HTMLAttributes<HTMLInputElement> {
	value?: string | number;
	onChange?: ChangeEventHandler<HTMLInputElement | HTMLTextAreaElement>;
	label?: ReactNode;
	error?: boolean | string;
	helperText?: ReactNode;
	fullWidth?: boolean;
	multiline?: boolean;
	minRows?: number;
	maxRows?: number;
	inputProps?: Record<string, unknown>;
	slotProps?: Record<string, any>;
	InputProps?: Record<string, any>;
	type?: string;
	sx?: SxProps;
	[key: string]: any;
}

export function LegacyTextField({
	label,
	value,
	onChange,
	error,
	helperText,
	fullWidth,
	multiline,
	minRows,
	maxRows: _maxRows,
	inputProps,
	slotProps,
	InputProps,
	sx,
	className,
	type = "text",
	...props
}: LegacyTextFieldProps) {
	const generatedId = useId();
	const id = props.id ?? generatedId;
	const htmlInput = {
		...(inputProps ?? {}),
		...(slotProps?.htmlInput ?? {}),
	};
	const descriptionId = `${id}-description`;
	const invalid = typeof error === "string" ? error.length > 0 : Boolean(error);
	const controlProps = omitCompatProps(props);
	const legacyStyle = sxToStyle(sx);
	const htmlInputStyle =
		typeof htmlInput.style === "object" && htmlInput.style !== null
			? (htmlInput.style as CSSProperties)
			: undefined;
	const controlStyle = {
		...htmlInputStyle,
		height: htmlInputStyle?.height ?? legacyStyle.height,
	};
	delete controlProps.margin;
	return (
		<label
			className={cn(
				"flex min-w-0 flex-col gap-1",
				fullWidth && "w-full",
				className,
			)}
			style={legacyStyle}
			htmlFor={id}
		>
			{label && (
				<span className="text-xs font-semibold text-slate-200">{label}</span>
			)}
			{multiline ? (
				<textarea
					{...controlProps}
					{...htmlInput}
					id={id}
					value={value ?? ""}
					onChange={onChange}
					rows={minRows ?? 3}
					aria-invalid={invalid || undefined}
					aria-describedby={helperText ? descriptionId : undefined}
					className={cn(
						"min-w-0 rounded-md border border-ui-border bg-slate-950/65 px-3 py-2 text-sm text-fg outline-hidden transition focus:border-compat-primary focus-visible:outline-2 focus-visible:outline-focus",
						invalid && "border-danger",
						controlProps.disabled && "cursor-not-allowed opacity-50",
						htmlInput.className,
					)}
					style={controlStyle}
				/>
			) : (
				<div
					className={cn(
						"flex h-9 min-w-0 items-center rounded-md border border-ui-border bg-slate-950/65 px-3 text-sm text-fg outline-hidden transition focus-within:border-compat-primary focus-within:outline-2 focus-within:outline-focus",
						invalid && "border-danger",
						controlProps.disabled && "cursor-not-allowed opacity-50",
					)}
					style={controlStyle}
				>
					{InputProps?.startAdornment}
					<input
						{...controlProps}
						{...htmlInput}
						id={id}
						type={type}
						value={value ?? ""}
						onChange={onChange}
						aria-invalid={invalid || undefined}
						aria-describedby={helperText ? descriptionId : undefined}
						className={cn(
							"h-full w-full bg-transparent text-sm text-fg outline-hidden placeholder:text-slate-600",
							htmlInput.className,
						)}
					/>
					{InputProps?.endAdornment}
				</div>
			)}
			{helperText && (
				<span
					id={descriptionId}
					className={cn("text-xs text-muted", invalid && "text-red-300")}
				>
					{helperText}
				</span>
			)}
		</label>
	);
}
