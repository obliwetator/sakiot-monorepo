/* tslint:disable */
/* eslint-disable */

/**
 * Copy-based prototype boundary. A production AudioWorklet may use the
 * WASM linear memory directly after profiling this simpler version.
 */
export class WasmSegmentProcessor {
    free(): void;
    [Symbol.dispose](): void;
    constructor(sample_rate: number, channels: number);
    process_interleaved(samples: Float32Array): boolean;
    /**
     * Offline clip path for length-changing rate/pitch transforms and
     * frame-order reverse. The returned buffer is newly allocated.
     */
    render_clip_interleaved(samples: Float32Array): Float32Array;
    reset(): void;
    /**
     * Apply a complete, versioned JavaScript configuration object. The
     * boundary intentionally rejects missing fields or unknown versions
     * instead of silently mixing schemas.
     */
    set_effect_config(config: any): boolean;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_wasmsegmentprocessor_free: (a: number, b: number) => void;
    readonly wasmsegmentprocessor_new: (a: number, b: number) => number;
    readonly wasmsegmentprocessor_process_interleaved: (a: number, b: number, c: number, d: any) => number;
    readonly wasmsegmentprocessor_render_clip_interleaved: (a: number, b: number, c: number) => [number, number];
    readonly wasmsegmentprocessor_reset: (a: number) => void;
    readonly wasmsegmentprocessor_set_effect_config: (a: number, b: any) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
