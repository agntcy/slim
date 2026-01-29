# Go Bindings Patch Script

## Overview

The `patch-bindings.sh` script fixes a critical bug in UniFFI-generated Go bindings related to async function error handling.

## The Problem

UniFFI's `uniffiRustCallAsync` function returns `(T, *E)` where `*E` is a pointer to the error type. When there's no error, it returns a typed nil pointer (`*SlimError(nil)`). 

In Go, when a typed nil pointer is assigned to an `error` interface variable, it becomes a **non-nil interface containing a nil pointer**. This causes `err != nil` checks to incorrectly detect errors even when there are none.

### Example of the Bug

```go
// Before patching (BROKEN):
func ConnectAsync(config ClientConfig) (uint64, error) {
    res, err := uniffiRustCallAsync[SlimError](...)
    return res, err  // err is *SlimError(nil), which becomes non-nil error interface!
}

// After patching (FIXED):
func ConnectAsync(config ClientConfig) (uint64, error) {
    res, errPtr := uniffiRustCallAsync[SlimError](...)
    
    if errPtr != nil {
        var _uniffiDefaultValue uint64
        return _uniffiDefaultValue, errPtr
    }
    return res, nil  // Explicitly return nil for error interface
}
```

## The Solution

The patch script:

1. Renames `err` to `errPtr` in async function calls to make it clear it's a pointer
2. Adds explicit nil checking before returning
3. Returns a proper `nil` error interface when there's no error
4. Returns the zero value of the return type when there is an error

## Usage

The script is automatically run as part of the `task generate` command. It will:

- Patch all async functions in `generated/slim_bindings/slim_bindings.go`
- Report how many functions were patched
- Verify the patch was applied correctly

### Manual Execution

```bash
cd data-plane/bindings/go
bash patch-bindings.sh
```

### Output

```
🔧 Patching generated Go bindings...
Patched 29 async functions
✅ Successfully patched async functions
   Total async functions: 29
   Functions with errPtr checks: 29
```

## Integration with Build Process

The patch is integrated into the Taskfile:

```yaml
generate:
  cmds:
    - task: copy-library
    - task: patch-header
    - task: patch-cgo-directives
    - task: patch-bindings  # ← Async error handling fix
```

## Technical Details

The script handles two types of async functions:

### Type 1: Functions returning (Type, error)

```go
func SomeAsync() (uint64, error) {
    res, errPtr := uniffiRustCallAsync[SlimError](...)
    
    if errPtr != nil {
        var _uniffiDefaultValue uint64
        return _uniffiDefaultValue, errPtr
    }
    return res, nil
}
```

### Type 2: Functions returning just error

```go
func SomeAsync() error {
    _, errPtr := uniffiRustCallAsync[SlimError](...)
    
    if errPtr != nil {
        return errPtr
    }
    return nil
}
```

## Related Issues

This is a well-known Go gotcha with interfaces and nil pointers. See:
- [Go FAQ: Why is my nil error value not equal to nil?](https://go.dev/doc/faq#nil_error)
- [Understanding nil in Go](https://go.dev/tour/methods/12)

## Similar Patches

This follows the same pattern as the Kotlin bindings patch (`data-plane/bindings/kotlin/patch-bindings.sh`), which fixes similar UniFFI-generated code issues.
