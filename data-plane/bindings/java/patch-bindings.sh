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
