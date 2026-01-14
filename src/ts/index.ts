import init, * as wasm from '../../pkg/runtime.js';
import wasmUrl from '../../pkg/runtime_bg.wasm?url';

export type Lang = 'c';

export type ExecutionResult = never;

export class Runtime {
  static async create(lang: Lang): Promise<void> {
    // const origin = window.location.origin;
    // const name = 'runtime';
    // const blob = new Blob([
    //   // 1) Import `pkg/runtime.js`, which has inside of it a `wasm_bindgen`
    //   // 2) expose the .wasm file
    //   `importScripts("${origin}/${name}.js");
    //    wasm_bindgen("${origin}/${name}_bg.wasm");`,
    // ]);

    // const url = URL.createObjectURL(blob);
    // const worker = new Worker(url);

    // worker.postMessage([2, 5]);
    // worker.onmessage = (result) => console.log(result);
    // throw new Error(lang);

    await init(wasmUrl);
    wasm.main();
    throw new Error(lang);
  }

  async run(): Promise<ExecutionResult> {
    throw new Error();
  }

  /* visit later: 
        runtime.stdout.pipeTo(console.log);
        runtime.stdin.write("haha ");
   */
}
