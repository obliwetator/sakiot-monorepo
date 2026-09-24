import { Search } from "lucide-react";

/** Text input with a leading search glyph, used by the browse sidebars. */
export function SearchInput(props: {
	id: string;
	label: string;
	placeholder: string;
	value: string;
	onChange: (value: string) => void;
}) {
	return (
		<label htmlFor={props.id} className="relative block">
			<Search
				aria-hidden="true"
				className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted"
			/>
			<input
				id={props.id}
				aria-label={props.label}
				value={props.value}
				onChange={(event) => props.onChange(event.currentTarget.value)}
				placeholder={props.placeholder}
				className="h-9 w-full rounded-md border border-ui-border bg-canvas pl-9 pr-3 text-sm text-fg outline-hidden placeholder:text-muted focus:border-primary focus-visible:outline-2 focus-visible:outline-primary focus-visible:outline-offset-1"
			/>
		</label>
	);
}
