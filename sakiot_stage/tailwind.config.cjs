/** @type {import('tailwindcss').Config} */
module.exports = {
	content: [
		"./public/**/*.html",
		"./src/**/*.{js,jsx,ts,tsx,vue}",
		"./index.html",
	],
	mode: "jit",
	darkMode: "media",
	// specify other options here
	theme: {
		// colors: {
		//   marcin: "#121212",
		// },
		extend: {
			colors: {
				marcin: "#121212",
			},
		},
	},
};
