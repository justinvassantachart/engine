/** Exact semantic fixtures shared by shipped, control and observer arms. */
export const fixtures = [
  { name: 'counter', source: 'int x=__COUNTER__; int y=__COUNTER__; int f(){return x+y;}\n', counter: true },
  { name: 'unused-definition', source: '#define CLOCK __TIME__\nint f(){return 1;}\n' },
  { name: 'unused-argument', source: '#define IGNORE(x) 1\nint f(){return IGNORE(__DATE__);}\n' },
  { name: 'inactive-conditional', source: '#if 0\nconst char* x=__TIMESTAMP__;\n#endif\nint f(){return 1;}\n' },
  ...['__DATE__', '__TIME__', '__TIMESTAMP__'].flatMap((builtin, index) => [
    { name: `temporal-direct-${index}`, source: `const char* x=${builtin};\n`, volatile: [builtin] },
    { name: `temporal-alias-${index}`, source: `#define CLOCK ${builtin}\nconst char* x=CLOCK;\n`, volatile: [builtin] },
    { name: `temporal-pasted-${index}`, source: `#define CAT(a,b) a##b\nconst char* x=CAT(${builtin.slice(0, 4)},${builtin.slice(4)});\n`, volatile: [builtin] },
    { name: `temporal-suppressed-${index}`, source: `#pragma clang diagnostic ignored "-Wdate-time"\nconst char* x=${builtin};\n`, volatile: [builtin] },
    { name: `temporal-redefined-${index}`, source: `#pragma clang diagnostic ignored "-Wbuiltin-macro-redefined"\n#undef ${builtin}\n#define ${builtin} "stable"\nconst char* x=${builtin};\n` },
    { name: `temporal-pop-${index}`, source: `#pragma clang diagnostic ignored "-Wbuiltin-macro-redefined"\n#pragma push_macro("${builtin}")\n#undef ${builtin}\n#define ${builtin} "stable"\nconst char* a=${builtin};\n#pragma pop_macro("${builtin}")\nconst char* b=${builtin};\n`, volatile: [builtin] },
  ]),
  { name: 'temporal-pragma', source: '#define APPLY(x) _Pragma(x)\nAPPLY(__DATE__)\nint f(){return 0;}\n', volatile: ['__DATE__'] },
  { name: 'temporal-conditional', source: '#if __TIME__\nint x=1;\n#endif\n', volatile: ['__TIME__'], error: true },
  { name: 'system-header-temporal', source: '#include <time-wrapper.h>\nconst char* x=SYSTEM_CLOCK;\n', volatile: ['__TIME__'] },
  { name: 'counter-backtrack', source: 'template<int> struct A {}; template<class T> int f(T){ return __COUNTER__; } int x=f(A<__COUNTER__>{});\n', counter: true },
  { name: 'positive-negative-probes', source: '#if __has_include("missing.h")\n#error unexpected\n#endif\n#if __has_include("present.h")\n#include "present.h"\n#endif\nint f(){return PRESENT;}\n', probes: ['missing.h', 'present.h'] },
  { name: 'selected-source-probe', source: '#if __has_include("sibling.cpp")\n#include "sibling.cpp"\n#endif\nint f(){return sibling;}\n', probes: ['sibling.cpp'] },
  { name: 'generated-output-probe', source: '#if __has_include("previous.o")\nint found=1;\n#else\nint found=0;\n#endif\n', probes: ['previous.o'], hostBypass: true },
  ...['data.bin', 'empty.bin', 'missing.bin'].map((name, index) => ({
    name: `embed-${index}`, source: `#pragma clang diagnostic ignored "-Wc23-extensions"\nunsigned char data[]={\n#embed "${name}"\n};\n`, unsupported: true, error: name === 'missing.bin',
  })),
  ...['data.bin', 'empty.bin', 'missing.bin'].map((name, index) => ({
    name: `has-embed-${index}`, source: `#if __has_embed("${name}")\nint found=1;\n#else\nint found=0;\n#endif\n`, unsupported: true,
  })),
  { name: 'global-incbin', source: 'asm(".incbin \\\"data.bin\\\"");\nint main(){return 0;}\n', unsupported: true },
  { name: 'function-assembly', source: 'void f(){asm volatile("nop");}\n', unsupported: true },
  { name: 'empty-memory-barrier', source: 'void f(){asm volatile("" ::: "memory");}\n' },
  { name: 'source-location', source: '#include <source_location>\nconst int line=std::source_location::current().line(); int f(){return line;}\n' },
  { name: 'warning', source: '#warning preserve this exact diagnostic\nint f(){return 1;}\n', error: true },
];
export const pchFixtures = [
  { name: 'pch-counter', header: '#include "present.h"\nint before=__COUNTER__;\n#define FROM_PCH __COUNTER__\n', source: 'int after=FROM_PCH; int f(){return before+after+PRESENT;}\n', counter: true },
  { name: 'pch-temporal-wrapper', header: '#define FROM_PCH __TIMESTAMP__\n', source: 'const char* after=FROM_PCH;\n', volatile: ['__TIMESTAMP__'] },
  { name: 'pch-imported-assembly', header: 'inline void from_pch(){asm volatile("nop");}\n', source: 'void f(){from_pch();}\n', unsupported: true },
];
