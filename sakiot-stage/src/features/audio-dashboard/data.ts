import type { Channels, Dirs, IndividualFileArray } from "../../Constants";

export function transform_to_months(data: Channels[]) {
	const byYear = new Map<
		number,
		Partial<Record<number, IndividualFileArray>>
	>();

	for (const channel of data) {
		for (const dirs of channel.dirs) {
			const months = dirs.months ?? {};
			for (const monthName of Object.keys(months)) {
				const month = parseInt(monthName, 10);
				const files = months[month];
				if (!files) continue;
				let yearEntry = byYear.get(dirs.year);
				if (!yearEntry) {
					yearEntry = {};
					byYear.set(dirs.year, yearEntry);
				}
				if (!yearEntry[month]) yearEntry[month] = [];
				for (const file of files) {
					// All of it: the tree item reads a removal from the entry too.
					yearEntry[month]?.push({ ...file, channel_id: channel.channel_id });
				}
			}
		}
	}

	for (const yearMonths of byYear.values()) {
		for (const key of Object.keys(yearMonths)) {
			const m = parseInt(key, 10);
			yearMonths[m]?.sort((a, b) => {
				const aTs = parseInt(a.file.split("-")[0] ?? "0", 10);
				const bTs = parseInt(b.file.split("-")[0] ?? "0", 10);
				return (
					(Number.isFinite(bTs) ? bTs : 0) - (Number.isFinite(aTs) ? aTs : 0)
				);
			});
		}
	}

	const sortedByYear: Dirs[] = [];
	byYear.forEach((value, key) => {
		sortedByYear.push({ year: key, months: value });
	});
	sortedByYear.sort((a, b) => b.year - a.year);
	return sortedByYear;
}
