import type {
	SharedDspWorkerRequest,
	SharedDspWorkerResponse,
} from "./sharedDspProtocol";
import {
	initializeSharedDspWorkerRuntime,
	preprocessSharedDspWorkerRequest,
	processSharedDspWorkerRequest,
} from "./sharedDspWorkerRuntime";

interface WorkerScope {
	onmessage: ((event: MessageEvent<SharedDspWorkerRequest>) => void) | null;
	postMessage(
		message: SharedDspWorkerResponse,
		transfer?: Transferable[],
	): void;
}

const scope = self as unknown as WorkerScope;

scope.onmessage = (event) => {
	const request = event.data;
	if (request.type === "initialize") {
		void initializeSharedDspWorkerRuntime()
			.then(() => scope.postMessage({ type: "ready" }))
			.catch((error: unknown) => {
				scope.postMessage({
					type: "error",
					message: error instanceof Error ? error.message : "DSP worker failed",
				});
			});
		return;
	}

	const operation =
		request.type === "preprocess"
			? preprocessSharedDspWorkerRequest(request).then((pcm) => ({
					type: "preprocessed" as const,
					id: request.id,
					pcm,
				}))
			: processSharedDspWorkerRequest(request).then((render) => ({
					type: "rendered" as const,
					id: request.id,
					render,
				}));
	void operation
		.then((response) => {
			const buffer =
				response.type === "preprocessed"
					? response.pcm.interleaved.buffer
					: response.render.pcm.interleaved.buffer;
			scope.postMessage(response, [buffer]);
		})
		.catch((error: unknown) => {
			scope.postMessage({
				type: "error",
				id: request.id,
				message: error instanceof Error ? error.message : "DSP render failed",
			});
		});
};
