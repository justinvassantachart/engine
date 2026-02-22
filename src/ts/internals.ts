/**
 * An exported symbol that allows us to define properties
 * on objects that are only available by library code and not consumers
 * of the library.
 */
export const Internals: unique symbol = Symbol();
