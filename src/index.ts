export type Lang = 'c';

export type ExecutionResult = {};

export class Runtime {
  static async create(lang: Lang): Promise<Runtime> {
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
