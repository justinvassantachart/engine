/**
 * Demo configuration: default source and file name used by the playground.
 * Keeps magic constants out of the main UI component.
 */

export const DEMO_SOURCE_FILE = 'main.c';

export const DEFAULT_SOURCE_CODE = `#include <iostream>

int ret1() {
  int x = 1;
  return x;
}
int main() {
  int x = 1;
  int y = ret1();
  int sum = y + 1;
  std::cout << (sum) << std::endl;
  return 0;
}
`;
