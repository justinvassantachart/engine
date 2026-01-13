#!bin/bash

# builds the Rust package
wasm-pack build --target web --out-dir=dist

# build the JavaScript
npm run build