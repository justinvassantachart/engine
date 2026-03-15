/**
 * C benchmark programs used to measure instrumentation overhead.
 * Each is designed to stress a different part of the runtime:
 *   - fib:    function-call heavy (highest expected overhead)
 *   - sort:   tight nested loops
 *   - sieve:  memory-access / branching
 *   - matmul: arithmetic / FP compute
 */

export type Benchmark = {
  id: string;
  label: string;
  description: string;
  code: string;
};

export const BENCHMARKS: Benchmark[] = [
  {
    id: 'fib',
    label: 'Fibonacci (recursive)',
    description: 'fib(38) — deeply recursive, ~39M function calls',
    code: `
#include <stdio.h>

int fib(int n) {
  if (n <= 1) return n;
  return fib(n - 1) + fib(n - 2);
}

int main() {
  printf("%d\\n", fib(38));
  return 0;
}
`.trim(),
  },
  {
    id: 'sort',
    label: 'Bubble Sort',
    description: 'Bubble-sort 6 000 ints — tight nested loops',
    code: `
#include <stdio.h>
#define N 6000

int main() {
  int a[N];
  for (int i = 0; i < N; i++) a[i] = N - i;
  for (int i = 0; i < N - 1; i++)
    for (int j = 0; j < N - 1 - i; j++)
      if (a[j] > a[j + 1]) {
        int t = a[j]; a[j] = a[j + 1]; a[j + 1] = t;
      }
  printf("%d\\n", a[N - 1]);
  return 0;
}
`.trim(),
  },
  {
    id: 'sieve',
    label: 'Sieve of Eratosthenes',
    description: 'Primes up to 2 000 000 — memory-access / branch heavy',
    code: `
#include <stdio.h>
#include <string.h>
#define N 2000000

int main() {
  static char s[N + 1];
  memset(s, 1, sizeof(s));
  s[0] = s[1] = 0;
  for (int i = 2; (long long)i * i <= N; i++)
    if (s[i])
      for (int j = i * i; j <= N; j += i)
        s[j] = 0;
  int count = 0;
  for (int i = 2; i <= N; i++) if (s[i]) count++;
  printf("%d\\n", count);
  return 0;
}
`.trim(),
  },
  {
    id: 'matmul',
    label: 'Matrix Multiply',
    description: '80×80 double matrix multiply — FP-arithmetic heavy',
    code: `
#include <stdio.h>
#define N 80

static double a[N][N], b[N][N], c[N][N];

int main() {
  for (int i = 0; i < N; i++)
    for (int j = 0; j < N; j++) {
      a[i][j] = i + j + 1;
      b[i][j] = (i == j) ? 1.0 : 0.0;
    }
  for (int i = 0; i < N; i++)
    for (int j = 0; j < N; j++) {
      c[i][j] = 0.0;
      for (int k = 0; k < N; k++)
        c[i][j] += a[i][k] * b[k][j];
    }
  printf("%.1f\\n", c[N - 1][N - 1]);
  return 0;
}
`.trim(),
  },
];
