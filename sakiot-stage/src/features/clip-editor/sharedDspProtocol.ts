import type { WaveformEnvelope } from "../audio-dashboard/waveformPeaks";
import type { SegmentEffects } from "./model";

export interface SharedDspPcm {
	channels: number;
	sampleRate: number;
	frames: number;
	interleaved: Float32Array;
}

export interface SharedDspRender {
	pcm: SharedDspPcm;
	peaks: WaveformEnvelope;
}

export interface SharedDspWorkerPreprocessRequest {
	type: "preprocess";
	id: number;
	sampleRate: number;
	left: Float32Array;
	right: Float32Array;
	effects: SegmentEffects;
}

export interface SharedDspWorkerProcessRequest {
	type: "process";
	id: number;
	pcm: SharedDspPcm;
	effects: SegmentEffects;
}

export type SharedDspWorkerDspRequest =
	| SharedDspWorkerPreprocessRequest
	| SharedDspWorkerProcessRequest;

export type SharedDspWorkerRequest =
	| { type: "initialize" }
	| SharedDspWorkerDspRequest;

export type SharedDspWorkerResponse =
	| { type: "ready" }
	| { type: "preprocessed"; id: number; pcm: SharedDspPcm }
	| { type: "rendered"; id: number; render: SharedDspRender }
	| { type: "error"; id?: number; message: string };
