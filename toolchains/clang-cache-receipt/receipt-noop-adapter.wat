;; Generic component wrappers do not request cache admission. This adapter has
;; no memory or effects; the engine supplies the real private compiler import.
(module
  (func (export "cache_receipt_v1") (param i32 i32) (result i32)
    i32.const 1))
