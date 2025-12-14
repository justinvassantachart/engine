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
000134 func[1] <add(int, int)>:
 000135: 01 7f                      | local[2] type=i32
 000137: 23 80 80 80 80 00          | global.get 0 <__stack_pointer>
 00013d: 41 10                      | i32.const 16
 00013f: 6b                         | i32.sub
 000140: 21 02                      | local.set 2
 000142: 20 02                      | local.get 2
 000144: 20 00                      | local.get 0
 000146: 36 02 0c                   | i32.store 2 12
 000149: 20 02                      | local.get 2
 00014b: 20 01                      | local.get 1
 00014d: 36 02 08                   | i32.store 2 8
 000150: 20 02                      | local.get 2
 000152: 20 02                      | local.get 2
 000154: 28 02 0c                   | i32.load 2 12
 000157: 20 02                      | local.get 2
 000159: 28 02 08                   | i32.load 2 8
 00015c: 6a                         | i32.add
 00015d: 36 02 04                   | i32.store 2 4
 000160: 20 02                      | local.get 2
 000162: 28 02 04                   | i32.load 2 4
 000165: 0f                         | return
 000166: 0b                         | end
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

      Local variables in WebAssembly are local to the function that is currently executing. Invoking another function and accessing its locals will access a different set of variables than the original function. Ultimately, it is up to the runtime (e.g. browser, Node.JS, etc.) that executes the bytecode to determine how these locals map onto hardware registers or memory.

      By convention, a function with `N` parameters will store its arguments in the local variables `0...N-1`. As we will see, this means that the value of `a` is stored in local `0` and the value of `b` is stored in local `1`.

6. **`local.get 2`**: Similarly to the previous instruction, this one fetches the value of local variable `2` and puts it back onto the OS.

7. **`local.get 0`**: This fetches the value of local `0` – corresponding to the first argument, `a`, to the `add` function – onto the OS.

8. **`i32.store 2 12`**: This instruction stores a value to memory. To do so, it pops two values off of the OS. The first value it pops is the value to be stored (in this case, the value of local `0`). The second value it pops is the base address where the value should be stored in memory (in this case, `__stack_pointer - 16`). You'll also notice that two parameters are encoded as immediates (baked into the instruction's binary):

    * `2` indicates that the alignment of the store will be 4 bytes (it is a power of two, `2^2=4`). In other words, it is a promise that the address we will store to is divisible by `4` bytes. This is not necessarily required, but is a hint to the runtime to help optimize this instruction.

    * `12` indicates that the value should be stored at an offset `12` above the base address. In other words, `(__stack_pointer - 16) + 12` or `__stack_pointer - 4`.


    The net result is that the value of `a` was stored at address `__stack_pointer - 4`.

9. **`local.get 2`**: Puts `__stack_pointer - 16` back onto the OS.

10. **`local.get 1`**: Puts `b` onto the OS.

11. **`i32.store 2 8`**: Stores `b` at address `__stack_pointer - 8`.

12. **`local.get 2`**: Puts `__stack_pointer - 16` onto the OS.

13. **`local.get 2`**: Puts `__stack_pointer - 16` onto the OS.

14. **`i32.load 2 12`**: This instruction pops an address from the OS and pushes the value loaded from memory at that address onto the OS. In this case, it loads the value stored by (8) – it loads `a`. At this point, the contents of the OS are `[__stack_pointer - 16, a]`.

15. **`local.get 2`**: Puts `__stack_pointer - 16` onto the OS.

16. **`i32.load 2 8`**: Loads the value (`b`) stored by (11) onto the OS. At this point, the contents of the OS are `[__stack_pointer - 16, a, b]`.

17. **`i32.add`**: Pops two values from the OS and adds them, pushing the result to the OS. The contents of the OS are `[__stack_pointer - 16, a + b]`.

18. **`i32.store 2 4`**: Stores `a + b` to `__stack_pointer - 12`.

19. **`local.get 2`**: Puts `__stack_pointer - 16` onto the OS.

20. **`i32.load 2 4`**: Loads `a + b` from memory.

21. **`return`**: Returns the value at the top of the OS to the caller and returns control to the caller. Notice that returning from a function in WebAssembly is simpler than in other instruction sets – no need to manage loading a return address or branching to a different PC value. In fact, WebAssembly has no concept of a program counter at all: the flow of execution is entirely managed by the runtime.


      Just as each WASM variable is typed, functions are typed with a signature as well. We can see the function signature for `add` in the `Function` section of the WASM binary from `wasm-objdump -x example.wasm`:

      ```cpp
      Type[3]:
        - type[0] () -> nil
        - type[1] (i32, i32) -> i32
        - type[2] () -> i32
      Function[4]:
        - func[0] sig=0 <__wasm_call_ctors>
        - func[1] sig=1 <add(int, int)>
      ```

      The function signature is indicated by the `sig=1` which references the signature type `(i32, i32) -> i32`. When `return` is executed, the function must at least as many arguments in the OS as expected by its return type and with the correct types (note that functions can return multiple values). If a function has more values than required on the OS when `return` is executed, the extra arguments are discarded.

      To call a function, the caller pushes the arguments onto the OS, which then become local variables in the caller. After execution, the arguments will have been popped and the results pushed onto the OS.

22. **`end`**: This function ends the structural block began by the function.

Notice that the above code has many peculiarities/inefficiencies that might be expected from compiling without optimizations. Enabling optimizations with `-O2` yields quite a different picture:

```
000184 func[1] <add(int, int)>:
 000185: 20 01                      | local.get 1
 000187: 20 00                      | local.get 0
 000189: 6a                         | i32.add
 00018a: 0b                         | end
```

You may also wonder how this WebAssembly code could execute fast (with near native speed) on the browser. Before it is executed, browsers compile the bytecode above into native instructions for the current platform (this is done, for example, by the [`WebAssembly.compile`](https://developer.mozilla.org/en-US/docs/WebAssembly/Reference/JavaScript_interface/compile_static) or [`WebAssembly.instantiateStreaming`](https://developer.mozilla.org/en-US/docs/WebAssembly/Reference/JavaScript_interface/instantiateStreaming_static) methods). During this time, the validity of the various WebAssembly constructs is checked (function signatures, well-typed arguments, etc.) and if any of them are ill-formed, a compilation error is thrown.

Read more about the [WebAssembly text format](https://developer.mozilla.org/en-US/docs/WebAssembly/Guides/Understanding_the_text_format) on the Mozilla docs.


# Debugging Basics

Modern compilers emit [DWARF info](https://dwarfstd.org/) about executables they compile. This info can be consumed by debuggers to allow introspection of the execution of a program at runtime. Let's take a tour of the DWARF info for the unoptimized `add` function compiled above.

## Line Info

One important role of a debugger is to analyze the execution of the program, operating in the target language (e.g. WebAssembly), in terms of **the source language**. To do this, the compiler must emit a mapping from original source text to the output assembly. This is contained in the `.debug_line` section of the DWARF output. This allows a debugger, for instance, to set a breakpoint on a line of code–it looks up the corresponding instruction associated with the line and stalls execution as soon as that instruction is reached.

Recall that compiling with debug info (`-g3`) embeds the debug info directly into a section of the executable. We can inspect this info using the `llvm-dwarfdump` tool which comes bundled with [`wasi-sdk`](https://github.com/WebAssembly/wasi-sdk). To view the debug line info, we can run `llvm-dwarfdump --debug-line example.wasm` to produce the following output:

```
Address            Line   Column File   ISA Discriminator OpIndex Flags
------------------ ------ ------ ------ --- ------------- ------- -------------
0x0000000000000005      1      0      1   0             0       0  is_stmt
0x0000000000000021      2     13      1   0             0       0  is_stmt prologue_end
0x0000000000000028      2     17      1   0             0       0 
0x000000000000002d      2     15      1   0             0       0 
0x000000000000002e      2      9      1   0             0       0 
0x0000000000000031      3     12      1   0             0       0  is_stmt
0x0000000000000036      3      5      1   0             0       0 
0x0000000000000038      3      5      1   0             0       0  end_sequence
```

This table shows which instructions in the output assembly correspond to which line/column offsets in the file. Note that the "Address" here corresponds to an offset from the start of the **Code** section of the WebAssembly binary. If we make the `wasm-objdump` output relative to the code section start[^code-section], we get:

[^code-section]: The output of `wasm-objdump` is relative to the start of the binary, not the code section start. We can find the start of the code section by inspecting `wasm-objdump -s example.wasm` to get the raw binary layout.

```
000005 func[1] <add(int, int)>:
 000006: 01 7f                      | local[2] type=i32
 000008: 23 80 80 80 80 00          | global.get 0 <__stack_pointer>
 00000e: 41 10                      | i32.const 16
 000010: 6b                         | i32.sub
 000011: 21 02                      | local.set 2
 000013: 20 02                      | local.get 2
 000015: 20 00                      | local.get 0
 000017: 36 02 0c                   | i32.store 2 12
 00001a: 20 02                      | local.get 2
 00001c: 20 01                      | local.get 1
 00001e: 36 02 08                   | i32.store 2 8
 000021: 20 02                      | local.get 2
 000023: 20 02                      | local.get 2
 000025: 28 02 0c                   | i32.load 2 12
 000028: 20 02                      | local.get 2
 00002a: 28 02 08                   | i32.load 2 8
 00002d: 6a                         | i32.add
 00002e: 36 02 04                   | i32.store 2 4
 000031: 20 02                      | local.get 2
 000033: 28 02 04                   | i32.load 2 4
 000036: 0f                         | return
 000037: 0b                         | end
```

Certain instructions in `.debug_line` are marked `is_stmt` to indicate the beginning of a statement: this information is used to mark the beginning of a group of instructions which logically correspond to a single statement or line in the code (and could be used by a visual debugger to denote a position where a breakpoint could be set). Notice also that instruction `21` is marked as `prologue_end` to indicate that this is the first line which proceeds the function prologue of `foo` – instructions which do not logically correspond to lines in the source but are needed to set up the function's stack frame (the compiler can also emit a related `epilogue_start`).

Recalling the original code:

```cpp
1. int add(int a, int b) {
2.     int c = a + b;
3.     return c;
4. }
```

If a user were to set a breakpoint on line 1, we might use `prologue_end` to automatically reposition their breakpoint to instruction `0x21` after the function prologue. Similarly, if a user placed a breakpoint on a line which has no corresponding instructions, we might reposition their breakpoint to the nearest line flagged with `is_stmt`.

From the line info for this example, we can identify two clear breakpoint instructions: `0x21` and `0x31`, corresponding to lines `1` and `2` in the program.

## Debug Info

The other crucial piece of information the compiler provides is the **debug info**, which contains info about each function, variable, and type in the source code and how it is structured or placed in memory. Let's take a look at a snippet of the debug info for our binary seen by running `llvm-dwarfdump --debug-info example.wasm`.

```
0x0000000b: DW_TAG_compile_unit
              DW_AT_producer	("clang version 21.1.4-wasi-sdk (https://github.com/llvm/llvm-project 222fc11f2b8f25f6a0f4976272ef1bb7bf49521d)")
              DW_AT_language	(DW_LANG_C_plus_plus_14)
              DW_AT_name	("example.cpp")
              DW_AT_stmt_list	(0x00000000)
              DW_AT_comp_dir	("/Users/jacob/Documents/wasm")
              DW_AT_low_pc	(0x00000005)
              DW_AT_high_pc	(0x00000038)

0x00000026:   DW_TAG_subprogram
                DW_AT_low_pc	(0x00000005)
                DW_AT_high_pc	(0x00000038)
                DW_AT_frame_base	(DW_OP_WASM_location 0x0 0x2, DW_OP_stack_value)
                DW_AT_linkage_name	("_Z3addii")
                DW_AT_name	("add")
                DW_AT_decl_file	("/Users/jacob/Documents/wasm/example.cpp")
                DW_AT_decl_line	(1)
                DW_AT_type	(0x0000006d "int")
                DW_AT_external	(true)

0x00000042:     DW_TAG_formal_parameter
                  DW_AT_location	(DW_OP_fbreg +12)
                  DW_AT_name	("a")
                  DW_AT_decl_file	("/Users/jacob/Documents/wasm/example.cpp")
                  DW_AT_decl_line	(1)
                  DW_AT_type	(0x0000006d "int")

0x00000050:     DW_TAG_formal_parameter
                  DW_AT_location	(DW_OP_fbreg +8)
                  DW_AT_name	("b")
                  DW_AT_decl_file	("/Users/jacob/Documents/wasm/example.cpp")
                  DW_AT_decl_line	(1)
                  DW_AT_type	(0x0000006d "int")

0x0000005e:     DW_TAG_variable
                  DW_AT_location	(DW_OP_fbreg +4)
                  DW_AT_name	("c")
                  DW_AT_decl_file	("/Users/jacob/Documents/wasm/example.cpp")
                  DW_AT_decl_line	(2)
                  DW_AT_type	(0x0000006d "int")

0x0000006c:     NULL

0x0000006d:   DW_TAG_base_type
                DW_AT_name	("int")
                DW_AT_encoding	(DW_ATE_signed)
                DW_AT_byte_size	(0x04)

0x00000074:   NULL
```

