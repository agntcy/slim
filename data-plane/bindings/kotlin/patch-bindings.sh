#!/usr/bin/env bash
set -euo pipefail

# patch-bindings.sh
# Patches UniFFI-generated Kotlin bindings to fix compatibility issues

BINDINGS_FILE="./generated/io/agntcy/slim/bindings/slim_bindings.kt"

echo "🔧 Patching generated Kotlin bindings..."

if [ ! -f "$BINDINGS_FILE" ]; then
  echo "❌ Error: Bindings file not found: $BINDINGS_FILE"
  exit 1
fi

# Fix exception classes with message parameter conflict
# UniFFI generates exception classes with a `message` parameter that conflicts
# with the inherited Throwable.message property in Kotlin
#
# Changes:
# 1. Rename constructor parameter from `message` to `msg`
# 2. Update the override property getter to use `msg`

sed -i.bak \
  -e 's/\(^[[:space:]]*\)val `message`: kotlin\.String$/\1val msg: kotlin.String/' \
  -e 's/get() = "message=${ `message` }"/get() = "message=${ msg }"/' \
  "$BINDINGS_FILE"

rm -f "$BINDINGS_FILE.bak"

# Fix wait() method conflict with Object.wait()
# UniFFI generates wait() methods that conflict with java.lang.Object.wait()
#
# Change: Rename wait() to waitForCompletion()

sed -i.bak \
  -e 's/fun `wait`()/fun waitForCompletion()/' \
  -e 's/\.`wait`()/.waitForCompletion()/' \
  "$BINDINGS_FILE"

rm -f "$BINDINGS_FILE.bak"

echo "✅ Successfully patched exception message parameters and wait() methods"