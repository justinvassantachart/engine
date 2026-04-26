int level3(int c) {
  return c;
}

int level2(int b) { return level3(b + 1); }
int level1(int a) { return level2(a + 1); }
int main() { return level1(1); }
