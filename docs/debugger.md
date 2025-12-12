# Introduction

## Compiling WebAssembly

WebAssembly (WASM) is a simple, portable bytecode designed to be a universal compilation target. The easiest way to understand it is to to take a look at some compiled examples. Consider the following `add` function in C++:

```cpp
int add(int a, int b) {
    int c = a + b;
    return c;
}
```

We can compile this code to WASM with the following command:

```bash
wasm32-wasip1-clang++ -nostdlib -g3 -Wl,--no-entry -Wl,--export-all example.cpp -o example.wasm
```

Take note of the different parts of the command:

* `wasm32-wasip1-clang++` is a version of `clang++` from the [`wasi-sdk`](https://github.com/WebAssembly/wasi-sdk) which compiles C/C++ to WASM. System calls (e.g. filesystem access) in the compiled binary obey [WASI](https://wasi.dev/) (WebAssembly System Interface) Preview 1.
* `-nostdlib` avoids linking the standard libraries (`libc`, `libc++`, `libunwind`). Ordinarily we would need to link against these when compiling a program which uses the C/C++ libraries, but we omit for now to produce a minimal binary.
* `-g3` includes full [DWARF](https://en.wikipedia.org/wiki/DWARF) debug info, which we will take a look at later.
* `-Wl,--no-entry` passes a linker flag indicating that our program will not have an entrypoint. In a WASI executable, a `_start` function must be provided that prepares the program for execution and eventually calls the `main` function. `_start` is usually provided by a `sysroot` (a collection of headers and libraries) that the linker links against, but we disregard this now.
* `-Wl,--export-all` passes a linker flag to export all functions in the compiled binary into the final WASM module. If we don't do this, we will end up with an empty module!
* `example.cpp -o example.wasm` compiles a file called `example.cpp` to produce an output binary called `example.wasm`.

You can find more info about the layout of the resulting binary [here](https://webassembly.github.io/spec/core/binary/index.html). Note that the DWARF debug info is embedded directly in the same binary containing the executable code.

## Inspecting the resulting bytecode

We can take a look at the resulting binary using `wasm-objdump`, one of the tools provided by [WABT](https://github.com/WebAssembly/wabt).

```bash
wasm-objdump -d example.wasm
```

`-d` shows the disassembly of the resulting WASM. Taking a look at the relevant section for the `add` function, we see:

```
000184 func[1] <add(int, int)>:
 000185: 01 7f                      | local[2] type=i32
 000187: 23 80 80 80 80 00          | global.get 0 <__stack_pointer>
 00018d: 41 10                      | i32.const 16
 00018f: 6b                         | i32.sub
 000190: 21 02                      | local.set 2
 000192: 20 02                      | local.get 2
 000194: 20 00                      | local.get 0
 000196: 36 02 0c                   | i32.store 2 12
 000199: 20 02                      | local.get 2
 00019b: 20 01                      | local.get 1
 00019d: 36 02 08                   | i32.store 2 8
 0001a0: 20 02                      | local.get 2
 0001a2: 20 02                      | local.get 2
 0001a4: 28 02 0c                   | i32.load 2 12
 0001a7: 20 02                      | local.get 2
 0001a9: 28 02 08                   | i32.load 2 8
 0001ac: 6a                         | i32.add
 0001ad: 36 02 04                   | i32.store 2 4
 0001b0: 20 02                      | local.get 2
 0001b2: 28 02 04                   | i32.load 2 4
 0001b5: 0f                         | return
 0001b6: 0b                         | end
```

This code is not very efficient and functions in a convoluted way, mostly because we didn't enable compiler optimizations. But it is a useful exercise to understand how WebAssembly works. Let's walk through the code one instruction at a time.

1. **`local[2] type=i32`**: This instruction declares a variable with type `i32` (a 32-bit integer). WebAssembly is a [typed language](https://webassembly.github.io/spec/core/syntax/types.html#syntax-numtype) with types like `i32`, `i64`, `f32`, `f64`, etc. Each function has a set of local variables it can access, each of which is typed. This instruction, part of the preamble of the function, indicates the type of a local variable which the function will make use of.
2. **`global.get 0 <__stack_pointer>`**: This instruction fetches the value of the global variable with index 0 and pushes it onto the **operand stack** (abbreviated OS), a store (somewhat like a set of registers) where data is temporarily stored before being operated on by other instructions. 

      Global variables in WebAssembly exist at the module (binary) level and are declared in advance in a dedicated section. We can see all the sections by running `wasm-objdump -x example.wasm`; in particular, notice the declaration of the `__stack_pointer` global variable with type `i32` and initial value `66560`:

      ```
      Global[11]:
        - global[0] i32 mutable=1 <__stack_pointer> - init i32=66560
      ```

      WASM does not have dedicated registers like traditional instruction sets–there is no `%sp` register which stores the stack pointer. Instead, `clang++` has decided to simulate a stack pointer by using a global variable. Note that in this context, "stack" refers to the call stack in linear memory and is different from "operand stack".

3. **`i32.const 16`**: This instruction pushes the immediate value `16` (of type `i32`) onto the top of the OS. The OS now has the last value of the `__stack_pointer` global variable and the value `16`.

4. **`i32.sub`**: This instruction consumes two `i32`s from the top of the OS, subtracts the first from the second, and pushes the result back onto the OS. After executing this instruction, the OS now has the value `__stack_pointer - 16`.
5. **`local.set 2`**: This instruction pops a value from the OS and stores it into the local variable with index 2.

      Local variables in WebAssembly are local to the function that is currently executing.