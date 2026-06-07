export function LiveDot() {
	return (
		<span
			role="img"
			className="ml-1.5 inline-block w-2 h-2 rounded-full bg-red-500 align-middle"
			aria-label="live"
			title="Live recording in progress"
		/>
	);
}

export function LivePill() {
	return (
		<span className="ml-2 inline-flex items-center px-1.5 py-0.5 text-[10px] font-bold text-white bg-red-600 rounded leading-none">
			● LIVE
		</span>
	);
}
