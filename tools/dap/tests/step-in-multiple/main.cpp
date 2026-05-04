void bar(int x) {  // line 1
    int y = x + 1; // line 2
  }                  // line 3
  
  void foo(int a) {  // line 5
    bar(a);        // line 6
    int b = 2;     // line 7
  }                  // line 8
  
  int main() {       // line 10
    foo(1);        // line 11
    int z = 3;     // line 12
  }                  // line 13