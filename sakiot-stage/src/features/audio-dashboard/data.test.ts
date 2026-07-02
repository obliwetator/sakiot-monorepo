import { describe, expect, it } from "bun:test";
import type { Channels } from "../../Constants";
import { transform_to_months } from "./data";

describe("transform_to_months", () => {
	it("groups files by year and month, injecting channel ids", () => {
		const data = [
			{
				channel_id: "channel-a",
				dirs: [
					{
						year: 2025,
						months: {
							1: [
								{
									file: "a.ogg",
									user_id: "user-a",
									display_name: "User A",
								},
							],
						},
					},
				],
			},
			{
				channel_id: "channel-b",
				dirs: [
					{
						year: 2026,
						months: {
							2: [
								{
									file: "b.ogg",
									user_id: "user-b",
									display_name: "User B",
								},
							],
						},
					},
					{
						year: 2025,
						months: {
							1: [
								{
									file: "c.ogg",
									user_id: "user-c",
									display_name: "User C",
								},
							],
						},
					},
				],
			},
		] as Channels[];

		expect(transform_to_months(data)).toEqual([
			{
				year: 2026,
				months: {
					2: [
						{
							channel_id: "channel-b",
							file: "b.ogg",
							user_id: "user-b",
							display_name: "User B",
						},
					],
				},
			},
			{
				year: 2025,
				months: {
					1: [
						{
							channel_id: "channel-a",
							file: "a.ogg",
							user_id: "user-a",
							display_name: "User A",
						},
						{
							channel_id: "channel-b",
							file: "c.ogg",
							user_id: "user-c",
							display_name: "User C",
						},
					],
				},
			},
		]);
	});
});
