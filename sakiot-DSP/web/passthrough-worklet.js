class SakiotPassthroughProcessor extends AudioWorkletProcessor {
	process(inputs, outputs) {
		const input = inputs[0] ?? [];
		const output = outputs[0] ?? [];
		for (let channel = 0; channel < output.length; channel += 1) {
			output[channel].set(input[channel] ?? input[0] ?? []);
		}
		return true;
	}
}

registerProcessor("sakiot-passthrough", SakiotPassthroughProcessor);
