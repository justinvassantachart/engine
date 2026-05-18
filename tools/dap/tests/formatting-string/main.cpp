#include <string>

int main() {
  std::string empty;
  std::string sso = "hello";
  std::string heap(64, 'x');
  std::string with_quotes = "he said \"hi\"\n";
  return (int)(empty.size() + sso.size() + heap.size() + with_quotes.size());
}
