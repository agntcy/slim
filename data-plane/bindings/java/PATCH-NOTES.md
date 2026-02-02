# Java Bindings Patch Notes

This document describes the patches applied to the UniFFI-generated Java bindings to fix compatibility and compilation issues.

## Overview

The `patch-bindings.sh` script automatically fixes several issues in the generated Java code from `uniffi-bindgen-java`. The script runs automatically as part of the `generate` task.

## Fixes Applied

### 1. JavaScript `===` Operator → Java `==`

**Problem:** UniFFI generator uses JavaScript's strict equality operator (`===`) which is not valid Java syntax.

**Location:** `equals()` methods in generated classes

**Fix:** Replace `this === other` with `this == other`

**Example:**
```java
// Before
if (this === other) {
    return true;
}

// After
if (this == other) {
    return true;
}
```

### 2. `wait()` Method Conflicts with `Object.wait()`

**Problem:** Generated `wait()` methods conflict with `java.lang.Object.wait()`, which is used for thread synchronization and can cause confusion.

**Location:** `CompletionHandle` and related interfaces

**Fix:** Rename `wait()` to `waitForCompletion()` throughout the codebase

**Example:**
```java
// Before
public void wait() throws SlimException { ... }
completion.wait();

// After
public void waitForCompletion() throws SlimException { ... }
completion.waitForCompletion();
```

### 3. `equals()` Return Type: `Boolean` → `boolean`

**Problem:** UniFFI generates `equals()` methods that return `Boolean` (wrapper) instead of `boolean` (primitive), which doesn't correctly override `Object.equals()`.

**Location:** All classes implementing custom equality

**Fix:** Change return type from `Boolean` to `boolean`

**Example:**
```java
// Before
@Override
public Boolean equals(Object other) { ... }

// After
@Override
public boolean equals(Object other) { ... }
```

### 4. Type Casting in `equals()` Method

**Problem:** The `lower()` method expects the specific type, but `equals()` receives `Object`. After the `instanceof` check, an explicit cast is needed.

**Location:** `equals()` implementations calling `FfiConverterType*.lower()`

**Fix:** Add explicit cast: `FfiConverterTypeFoo.INSTANCE.lower((Foo)other)`

**Example:**
```java
// Before
FfiConverterTypeName.INSTANCE.lower(other)

// After
FfiConverterTypeName.INSTANCE.lower((Name)other)
```

### 5. Duplicate Pattern Variable Names in Sealed Types

**Problem:** When sealed interface records have fields named "value", the pattern variable in switch expressions conflicts with the field name, causing compilation errors.

**Location:** `FfiConverterTypeJwtKeyData` and similar converter classes

**Fix:** Rename pattern variable from `value` to `v` to avoid conflict

**Example:**
```java
// Before
case JwtKeyData.Data(var value) ->
    (4L + FfiConverterString.INSTANCE.allocationSize(value));

// After
case JwtKeyData.Data(var v) ->
    (4L + FfiConverterString.INSTANCE.allocationSize(v));
```

## Running the Patch Script

The patch script runs automatically during code generation:

```bash
task generate
```

To run it manually on already-generated code:

```bash
./patch-bindings.sh
```

## Reporting Issues

These patches work around bugs in `uniffi-bindgen-java`. If you encounter similar issues, consider:

1. Updating the patch script to handle new cases
2. Reporting bugs upstream to: https://github.com/IronCoreLabs/uniffi-bindgen-java

## Version Information

- **uniffi-bindgen-java**: Installed from git (IronCoreLabs/uniffi-bindgen-java)
- **Patch Script**: `patch-bindings.sh` in this directory
- **Last Updated**: 2026-02-02
