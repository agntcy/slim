#!/usr/bin/env bash
set -euo pipefail

# patch-bindings.sh
# Patches UniFFI-generated Java bindings to fix compatibility issues

BINDINGS_DIR="generated/uniffi"

echo "🔧 Patching generated Java bindings..."

if [ ! -d "$BINDINGS_DIR" ]; then
  echo "❌ Error: Bindings directory not found: $BINDINGS_DIR"
  exit 1
fi

# Fix 1: JavaScript === operator instead of Java ==
# UniFFI generates === which is not valid Java syntax
#
# Changes:
# - Replace `this === other` with `this == other` in equals() methods
echo "  → Fixing JavaScript === operator..."
find "$BINDINGS_DIR" -name "*.java" -type f -exec sed -i.bak \
  -e 's/this === other/this == other/g' \
  {} \;

# Fix 2: wait() method conflict with Object.wait()
# UniFFI generates wait() methods that conflict with java.lang.Object.wait()
# This can cause issues with thread synchronization
#
# Changes:
# - Rename wait() to waitForCompletion() in method declarations
# - Update method calls from .wait() to .waitForCompletion()
# - Update documentation references
echo "  → Fixing wait() method conflicts with Object.wait()..."
find "$BINDINGS_DIR" -name "*.java" -type f -exec sed -i.bak \
  -e 's/public void wait()/public void waitForCompletion()/g' \
  -e 's/\.wait()/\.waitForCompletion()/g' \
  -e 's/completion\.wait()?;/completion.waitForCompletion()?;/g' \
  -e 's/Call `\.wait()`/Call `.waitForCompletion()`/g' \
  {} \;

# Fix 3: equals() method return type
# UniFFI generates equals() returning Boolean (wrapper) instead of boolean (primitive)
# Java's Object.equals() must return primitive boolean
#
# Changes:
# - Change return type from Boolean to boolean
echo "  → Fixing equals() return type..."
find "$BINDINGS_DIR" -name "*.java" -type f -exec sed -i.bak \
  -e 's/public Boolean equals(Object other)/public boolean equals(Object other)/g' \
  {} \;

# Fix 4: equals() method - cast Object to correct type for lower()
# After instanceof check, we need to cast the Object parameter to the specific type
# for the lower() method call
echo "  → Fixing equals() type casting..."
# Look for patterns like: FfiConverterTypeFoo.INSTANCE.lower(other)
# and replace with: FfiConverterTypeFoo.INSTANCE.lower((Foo)other)
find "$BINDINGS_DIR" -name "*.java" -type f -exec sed -i.bak \
  -e 's/\(FfiConverterType\([A-Za-z]*\)\.INSTANCE\.lower(\)other\()\)/\1(\2)other\3/g' \
  {} \;

# Fix 5: Duplicate variable names in pattern matching for sealed types
# When sealed interface records have fields named "value", the pattern variable 
# conflicts with the field name in switch expressions
#
# Changes:
# - Rename pattern variable from `value` to `v` when it conflicts
echo "  → Fixing duplicate pattern variable names in sealed type switches..."

# Fix for FfiConverterTypeJwtKeyData specifically
if [ -f "$BINDINGS_DIR/io/agntcy/slim/bindings/FfiConverterTypeJwtKeyData.java" ]; then
  sed -i.bak \
    -e 's/case JwtKeyData\.Data(var value)/case JwtKeyData.Data(var v)/g' \
    -e 's/\+ FfiConverterString\.INSTANCE\.allocationSize(value)/+ FfiConverterString.INSTANCE.allocationSize(v)/g' \
    -e 's/FfiConverterString\.INSTANCE\.write(value, buf)/FfiConverterString.INSTANCE.write(v, buf)/g' \
    "$BINDINGS_DIR/io/agntcy/slim/bindings/FfiConverterTypeJwtKeyData.java"
fi

# Fix 7: RpcCode and similar enums - Rust-style unsigned literals and malformed delimiters
# UniFFI generates 0u, 1u etc. (Rust) and ,} / ;} instead of , / ;
# Also cast int literals to Short for enum constructors expecting Short
echo "  → Fixing enum unsigned literals and delimiters..."
find "$BINDINGS_DIR" -name "*.java" -type f -exec sed -i.bak \
  -e 's/\([0-9][0-9]*\)u/\1/g' \
  -e 's/,}/,/g' \
  -e 's/;}/;/g' \
  {} \;
# RpcCode enum constructor expects Short; cast int literals
if [ -f "$BINDINGS_DIR/io/agntcy/slim/bindings/RpcCode.java" ]; then
  sed -i.bak -e 's/\([A-Z_][A-Z_0-9]*\)(\([0-9][0-9]*\))/\1((short)\2)/g' \
    "$BINDINGS_DIR/io/agntcy/slim/bindings/RpcCode.java"
fi

# Fix 8: Missing java.util.List / java.util.ArrayList imports
# UniFFI generates List<String> fields for MulticastSessionClosed but does not
# add the required import statements. The converter file also needs ArrayList.
# Some generated files have no existing imports, so we insert after the package line.
echo "  → Adding missing java.util.List imports..."
find "$BINDINGS_DIR" -name "*.java" -type f -exec grep -l 'List<' {} \; | while read -r file; do
  if ! grep -q 'import java.util.List;' "$file"; then
    awk '/^package / { print; print ""; print "import java.util.List;"; next } {print}' "$file" > "$file.tmp" && mv "$file.tmp" "$file"
  fi
done
find "$BINDINGS_DIR" -name "*.java" -type f -exec grep -l 'ArrayList<' {} \; | while read -r file; do
  if ! grep -q 'import java.util.ArrayList;' "$file"; then
    awk '/^package / { print; print ""; print "import java.util.ArrayList;"; next } {print}' "$file" > "$file.tmp" && mv "$file.tmp" "$file"
  fi
done

# Fix 9: ResponseSink close() conflict with AutoCloseable
# ResponseSink implements both AutoCloseable.close() and ResponseSinkInterface.close()
# Rename the stream close to closeStream() to avoid duplicate method
echo "  → Fixing ResponseSink close() conflict with AutoCloseable..."
find "$BINDINGS_DIR" -name "*.java" -type f -exec sed -i.bak \
  -e 's/public void close() throws RpcException/public void closeStream() throws RpcException/g' \
  -e 's/sink\.close()/sink.closeStream()/g' \
  {} \;

# Fix 6: UniFFI contract version mismatch  
# uniffi-bindgen-java 0.2.1 generates contract version 29, but needs to match Rust uniffi version
# After testing, Rust uniffi 0.28.3 uses contract version 29, not 28
#
# Changes:
# - Ensure bindingsContractVersion matches whatever Rust is using (keep at 29)
echo "  → Fixing UniFFI contract version (if needed)..."
# Note: No change needed - both are 29
# if [ -f "$BINDINGS_DIR/io/agntcy/slim/bindings/NamespaceLibrary.java" ]; then
#   sed -i.bak \
#     -e 's/int bindingsContractVersion = 29;/int bindingsContractVersion = 29;/g' \
#     "$BINDINGS_DIR/io/agntcy/slim/bindings/NamespaceLibrary.java"
# fi

# Clean up backup files
find "$BINDINGS_DIR" -name "*.java.bak" -type f -delete

echo "✅ Successfully patched Java bindings:"
echo "   - Fixed === operator (JavaScript syntax) → =="
echo "   - Renamed wait() → waitForCompletion() to avoid Object.wait() conflict"
echo "   - Fixed equals() return type: Boolean → boolean"
echo "   - Fixed equals() Object casting for lower() method"
echo "   - Fixed duplicate pattern variable names in sealed type switches"
echo "   - Fixed enum unsigned literals (0u → 0), delimiters (,} → ,; ;} → ;), and Short cast for RpcCode"
echo "   - Added missing java.util.List/ArrayList imports where needed"
echo "   - Renamed ResponseSink.close() → closeStream() to avoid AutoCloseable conflict"
