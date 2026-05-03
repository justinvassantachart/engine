struct Compound {
  long a, b;
};

int main() {
  Compound c { 1, 2};

  Compound& cref = c; 
  Compound* cptr = &c;
  Compound* null = nullptr;

  long& lref = c.a;
  long* lptr = &c.a;

  return 0;
}