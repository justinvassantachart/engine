#include <chrono>

namespace custom {
struct Widget {
  using Value = int;
  Value value;
};

typedef Widget WidgetTypedef;
using WidgetUsing = Widget;
}  // namespace custom

int main() {
  std::chrono::time_point<std::chrono::system_clock, std::chrono::duration<long long, std::ratio<1, 1000000000>>> deep_chrono{};
  custom::Widget namespaced{1};
  custom::WidgetTypedef via_typedef{2};
  custom::WidgetUsing via_using{3};
  custom::Widget::Value via_nested_using{4};
  return 0;
}
