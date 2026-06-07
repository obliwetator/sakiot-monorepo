export function parseFileName(file: string): {
	time: string;
	userId: string;
	username: string;
} {
	const base = file.replace(/\.ogg$/, "");
	const parts = base.split("-");
	const ts = parseInt(parts[0] ?? "", 10);
	const userId = parts[1] ?? "";
	const username = parts.slice(2).join("-");
	let time = base;
	if (Number.isFinite(ts)) {
		const d = new Date(ts);
		const pad = (n: number) => n.toString().padStart(2, "0");
		time = `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
	}
	return { time, userId, username };
}
