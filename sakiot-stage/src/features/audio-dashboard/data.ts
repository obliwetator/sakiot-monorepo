import type {
	Channels,
	Dirs,
	IndividualFile,
	IndividualFileArray,
	MonthNumber,
} from "../../Constants";

interface Super {
	channel_id: string;
	year: number;
	file: IndividualFile;
	month: MonthNumber;
}

// TODO: We get the data organized by channels. The data is easier to work with when it is organized by years and the channel_id is used as metadata of sorts
// FIXME: There should definitely be a better way to do it
export function transform_to_months(data: Channels[]) {
	const all_stuff: Super[] = [];

	// Extract all elements in a an array

	// Loops over channels
	data.forEach((channel) => {
		// Loops over the years in the channel
		channel.dirs.forEach((dirs) => {
			const months = dirs.months ?? {};
			const months_obj = Object.keys(months);
			// Loops over the months in the year of the channel
			months_obj.forEach((month_name) => {
				const month = parseInt(month_name, 10) as MonthNumber;
				// In JavaScript, object keys are strings. Using month_name works reliably.
				const files = months[month];

				if (!files) return;

				const all_file: IndividualFileArray = [];

				files.forEach((file: IndividualFile) => {
					const indi: IndividualFile = {
						channel_id: channel.channel_id,
						file: file.file,
						user_id: file.user_id,
						display_name: file.display_name,
						recording_session_id: file.recording_session_id,
						channel_journey: file.channel_journey,
						state: file.state,
						access: file.access,
					};
					all_file.push(indi);

					all_stuff.push({
						channel_id: channel.channel_id,
						file: indi,
						year: dirs.year,
						month: month,
					});
				});
			});
		});
	});

	const hashmap3 = new Map<
		number,
		Partial<Record<MonthNumber, IndividualFileArray>>
	>();
	const hashmap_month: Partial<Record<MonthNumber, IndividualFileArray>> = {};

	const sorted_by_year: Dirs[] = [];

	// Arrannge elements by year in a Map
	all_stuff.forEach((value) => {
		if (!hashmap3.has(value.year)) hashmap3.set(value.year, {});

		if (!hashmap_month[value.month]) hashmap_month[value.month] = [];
		hashmap_month[value.month]?.push({
			channel_id: value.channel_id,
			file: value.file.file,
			user_id: value.file.user_id,
			display_name: value.file.display_name,
			recording_session_id: value.file.recording_session_id,
			channel_journey: value.file.channel_journey,
			state: value.file.state,
			access: value.file.access,
		});

		const dirs = hashmap3.get(value.year);
		if (!dirs) return;

		if (!dirs[value.month]) dirs[value.month] = [];

		dirs[value.month]?.push({
			channel_id: value.channel_id,
			file: value.file.file,
			user_id: value.file.user_id,
			display_name: value.file.display_name,
			recording_session_id: value.file.recording_session_id,
			channel_journey: value.file.channel_journey,
			state: value.file.state,
			access: value.file.access,
		});
	});

	// Transform Map into object
	hashmap3.forEach((value, key) => {
		sorted_by_year.push({ year: key, months: value });
	});

	// Sort by years (descending)
	sorted_by_year.sort((a, b) => b.year - a.year);

	return sorted_by_year;
}
