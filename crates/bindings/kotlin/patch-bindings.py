#!/usr/bin/env python3
"""
patch-bindings.py
Patches UniFFI-generated Kotlin bindings to fix compatibility issues
"""

import re
import sys

BINDINGS_FILE = "./generated/io/agntcy/slim/bindings/slim_bindings.kt"

def main():
    print("🔧 Patching generated Kotlin bindings...")
    
    try:
        with open(BINDINGS_FILE, 'r') as f:
            content = f.read()
    except FileNotFoundError:
        print(f"❌ Error: Bindings file not found: {BINDINGS_FILE}")
        sys.exit(1)
    
    # Fix 1: Exception classes with message parameter conflict
    # Rename `message` parameter to msg (with or without trailing comma)
    content = re.sub(
        r'val `message`: kotlin\.String(,?)',
        r'val msg: kotlin.String\1',
        content
    )
    
    # Fix 2: Update references to `message` in exception message strings
    content = re.sub(
        r'message=\$\{ `message` \}',
        r'message=${ msg }',
        content
    )
    
    # Fix 3: wait() method conflict with Object.wait()
    content = re.sub(
        r'fun `wait`\(\)',
        'fun waitForCompletion()',
        content
    )
    content = re.sub(
        r'\.`wait`\(\)',
        '.waitForCompletion()',
        content
    )
    
    # Fix 4: close() method conflict with AutoCloseable.close()
    content = re.sub(
        r'override fun `close`\(\)',
        'fun closeStream()',
        content
    )
    content = re.sub(
        r'(@Throws\([^)]*\))override suspend fun `closeAsync`\(\)',
        r'\1suspend fun closeStreamAsync()',
        content
    )
    
    with open(BINDINGS_FILE, 'w') as f:
        f.write(content)
    
    print("✅ Successfully patched exception message parameters, wait() methods, and close() conflicts")

if __name__ == "__main__":
    main()
