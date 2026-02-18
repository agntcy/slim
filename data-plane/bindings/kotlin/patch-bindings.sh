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
# 1. Rename constructor parameter from `message` to msg (handle both with and without trailing comma, with backticks)
# 2. Update all references to `message` to use msg in the exception message strings

sed -i.bak \
  -e 's/val `message`: kotlin\.String\(,\?\)/val msg: kotlin.String\1/' \
  -e 's/message=${ `message` }/message=${ msg }/' \
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

# Fix close() method conflict
# UniFFI generates a close() method that conflicts with AutoCloseable.close()
# The AutoCloseable.close() calls destroy(), while the UniFFI close() is a separate method
#
# Change: Rename the UniFFI-generated close() method to closeStream()

sed -i.bak \
  -e 's/override fun `close`()/fun closeStream()/' \
  -e 's/\(@Throws([^)]*)\)override suspend fun `closeAsync`()/\1suspend fun closeStreamAsync()/' \
  "$BINDINGS_FILE"

rm -f "$BINDINGS_FILE.bak"

echo "✅ Successfully patched exception message parameters, wait() methods, and close() conflicts"