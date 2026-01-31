#!/usr/bin/env bash
set -euo pipefail

# patch-bindings.sh  
# Patches UniFFI-generated Go bindings to fix async function error handling
#
# Problem: uniffiRustCallAsync returns (T, *E) where *E is a typed nil pointer
# when there's no error. When this typed nil is assigned to an error interface,
# it becomes a non-nil interface containing a nil pointer, causing err != nil
# checks to incorrectly detect errors.
#
# Solution: Check the error pointer before returning and explicitly return nil
# for the error interface when there's no error.

BINDINGS_FILE="generated/slim_bindings/slim_bindings.go"

echo "🔧 Patching generated Go bindings..."

if [ ! -f "$BINDINGS_FILE" ]; then
  echo "❌ Error: Bindings file not found: $BINDINGS_FILE"
  exit 1
fi

# Create a backup
cp "$BINDINGS_FILE" "$BINDINGS_FILE.bak"

# Use Python for the transformation
python3 << 'PYTHON_EOF'
import re
import sys

# Read the file
with open("generated/slim_bindings/slim_bindings.go", "r") as f:
    content = f.read()
    lines = content.split('\n')

# First pass: rename err to errPtr
content = re.sub(r'(res|_uniffiRV|_), err := uniffiRustCallAsync\[', r'\1, errPtr := uniffiRustCallAsync[', content)

# Write intermediate result
lines = content.split('\n')
patches = 0

# Second pass: find and fix async functions
i = 0
while i < len(lines):
    line = lines[i]
    
    # Check if this is an Async function
    if re.match(r'^func \(_self \*\w+\) \w+Async\(', line):
        # Get return type
        match_type = re.search(r'\) \(([^,)]+), error\) \{', line)
        match_error_only = re.search(r'\) error \{', line)
        
        if match_type:
            return_type = match_type.group(1)
            is_error_only = False
        elif match_error_only:
            return_type = None
            is_error_only = True
        else:
            i += 1
            continue
        
        # Find uniffiRustCallAsync and return in this function
        func_start = i
        var_name = None
        return_line = None
        
        # Scan the function
        j = i + 1
        brace_count = 1  # We've seen the opening brace
        while j < len(lines) and brace_count > 0:
            # Count braces to know when function ends
            brace_count += lines[j].count('{') - lines[j].count('}')
            
            # Find the async call
            if var_name is None:
                match = re.match(r'\t(res|_uniffiRV|_), errPtr := uniffiRustCallAsync\[', lines[j])
                if match:
                    var_name = match.group(1)
            
            # Find the return statement
            if var_name:
                if not is_error_only and re.match(rf'\treturn {re.escape(var_name)}, (err|errPtr)$', lines[j]):
                    return_line = j
                    break
                elif is_error_only and re.match(r'\treturn (err|errPtr)$', lines[j]):
                    return_line = j
                    break
            
            if brace_count == 0:
                break
            j += 1
        
        # If we found both the call and the return, patch it
        if var_name and return_line is not None:
            # Check if already patched
            if return_line > 1 and 'if errPtr != nil' not in lines[return_line - 2]:
                if not is_error_only:
                    # Replace the return statement
                    lines[return_line] = f'\tif errPtr != nil {{\n\t\tvar _uniffiDefaultValue {return_type}\n\t\treturn _uniffiDefaultValue, errPtr\n\t}}\n\treturn {var_name}, nil'
                else:
                    lines[return_line] = f'\tif errPtr != nil {{\n\t\treturn errPtr\n\t}}\n\treturn nil'
                patches += 1
    
    i += 1

# Join back and write
content = '\n'.join(lines)
with open("generated/slim_bindings/slim_bindings.go", "w") as f:
    f.write(content)

print(f"Patched {patches} async functions", file=sys.stderr)
PYTHON_EOF

PYTHON_EXIT=$?

if [ $PYTHON_EXIT -eq 0 ]; then
  # Verify no unpatched functions remain
  if grep -q "return res, err$" "$BINDINGS_FILE" 2>/dev/null || grep -q "return _uniffiRV, err$" "$BINDINGS_FILE" 2>/dev/null; then
    echo "❌ Error: Some functions still have 'return ..., err' pattern"
    grep -n "return res, err$\|return _uniffiRV, err$" "$BINDINGS_FILE" || true
    mv "$BINDINGS_FILE.bak" "$BINDINGS_FILE"
    exit 1
  fi
  
  PATCHED_COUNT=$(grep -c "if errPtr != nil {" "$BINDINGS_FILE" || true)
  ASYNC_COUNT=$(grep -c "uniffiRustCallAsync\[" "$BINDINGS_FILE" || true)
  
  echo "✅ Successfully patched async functions"
  echo "   Total async functions: $ASYNC_COUNT"
  echo "   Functions with errPtr checks: $PATCHED_COUNT"
  rm -f "$BINDINGS_FILE.bak"
else
  echo "❌ Error: Patch script failed. Restoring backup."
  mv "$BINDINGS_FILE.bak" "$BINDINGS_FILE"
  exit 1
fi
