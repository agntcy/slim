# SLIM JavaScript Bindings

JavaScript/TypeScript bindings for SLIM generated from Rust using uniffi-bindgen-react-native.

## Generate Bindings

```bash
# Install dependencies
npm install

# Generate bindings (builds Rust, generates TS, compiles JS)
task generate

# Or manually:
cd ../adapter && cargo build --release --lib
cd ../javascript
mkdir -p generated/src generated/cpp
cd ../adapter && ../javascript/node_modules/.bin/ubrn generate jsi bindings \
  --library \
  --ts-dir ../javascript/generated/src \
  --cpp-dir ../javascript/generated/cpp \
  ../../target/release/libslim_bindings.a
cd ../javascript
sed -i '' 's/async public /public async /g' generated/src/slim_bindings.ts
npm run build
```

## Output

- `generated/src/` - TypeScript bindings
- `generated/cpp/` - C++ JSI bindings for React Native
- `dist/` - Compiled JavaScript with type definitions
