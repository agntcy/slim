#!/usr/bin/env tsx
// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/**
 * Post-generation patcher for Node.js bindings
 *
 * Applies FFI compatibility fixes to generated code to bridge the gap between:
 * - uniffi-bindgen-node (generates React Native JSI types with BigInt pointers)
 * - ffi-rs (expects JavaScript Number for pointers)
 * - UniFFI 0.31+ Rust scaffolding exports flat symbols (uniffi_<crate>_fn_...);
 *   the pinned bindgen still emits module-qualified names (uniffi_<crate>__<mod>_fn_...).
 *   We flatten those prefixes so dlsym resolves the same symbols as the .dylib/.so.
 *
 * ---------------------------------------------------------------------------
 * Upstream pin (must match Taskfile `install-uniffi-bindgen-node`):
 *   https://github.com/livekit/uniffi-bindgen-node  tag  uniffi-bindgen-node@0.1.4
 *
 * If patches fail validation after upgrading the generator, re-run against
 * fresh output and adjust patterns; consider opening an issue upstream so
 * these patches can be removed long-term:
 *   https://github.com/livekit/uniffi-bindgen-node/issues
 * ---------------------------------------------------------------------------
 */

import * as fs from 'fs';
import * as path from 'path';

const GENERATED_DIR = process.argv[2] || path.join(__dirname, '../generated');
const BINDINGS_FILE = path.join(GENERATED_DIR, 'slim-bindings-node.ts');
const SYS_FILE = path.join(GENERATED_DIR, 'slim-bindings-sys.ts');

const IMPORT_MARKER = "} from './slim-bindings-sys';";

console.log('🔧 Applying FFI compatibility patches...');
console.log(`   Generated dir: ${GENERATED_DIR}`);

/** Rust UniFFI 0.31+ uses flat export names; uniffi-bindgen-node still prints module paths. */
function patchFlattenUniffiExportSymbols(content: string, fileLabel: string): string {
  console.log(`  → Flatten UniFFI export symbols (${fileLabel})...`);
  const hierarchical =
    /uniffi_slim_bindings((?:__[a-z][a-z0-9_]*)+)(?=_fn_|_checksum_)/g;
  const matches = content.match(hierarchical);
  if (!matches || matches.length === 0) {
    if (content.includes('uniffi_slim_bindings_fn_func_initialize_with_defaults')) {
      console.log('     Already flat (idempotent)');
      return content;
    }
    throw new Error(
      `Flatten: expected hierarchical uniffi_slim_bindings__* symbols in ${fileLabel}`
    );
  }
  const result = content.replace(hierarchical, 'uniffi_slim_bindings');
  console.log(`     Flattened ${matches.length} symbol prefix occurrence(s)`);
  return result;
}

// Patch 1: FfiConverterBytes -> FfiConverterArrayBuffer
function patch1_FixConverterName(content: string): string {
  console.log('  → Patch 1: FfiConverterBytes references...');
  const before = (content.match(/FfiConverterBytes/g) || []).length;
  const result = content.replace(/FfiConverterBytes/g, 'FfiConverterArrayBuffer');
  if (before === 0 && !result.includes('FfiConverterArrayBuffer')) {
    throw new Error(
      'Patch 1: expected FfiConverterBytes or FfiConverterArrayBuffer in generator output. ' +
        'uniffi-bindgen-node output may have changed (see file header for pinned tag).'
    );
  }
  if (before > 0) {
    console.log(`     Replaced ${before} occurrence(s)`);
  } else {
    console.log('     No FfiConverterBytes (already using ArrayBuffer or idempotent run)');
  }
  return result;
}

// Patch 2: Inject toFfiPointer helper function
function patch2_InjectHelper(content: string): string {
  console.log('  → Patch 2: Inject toFfiPointer helper...');
  if (!content.includes(IMPORT_MARKER) && !content.includes('function toFfiPointer')) {
    throw new Error(
      `Patch 2: missing import marker ${JSON.stringify(IMPORT_MARKER)} and no existing toFfiPointer().`
    );
  }
  if (content.includes('function toFfiPointer')) {
    return content;
  }
  return content.replace(IMPORT_MARKER, IMPORT_MARKER + patch2HelperBlock());
}

function patch2HelperBlock(): string {
  return `

// ==========
// FFI Compatibility Helpers
// ==========

/**
 * Convert pointer value to Number for ffi-rs compatibility.
 * React Native JSI uses BigInt for pointers, but ffi-rs expects Number.
 */
function toFfiPointer(ptr: UnsafeMutableRawPointer): number {
  return typeof ptr === "bigint" ? Number(ptr) : ptr;
}
`;
}

// Patch 3: Wrap clonePointer(this) calls with toFfiPointer
function patch3_WrapClonePointerThis(content: string): string {
  console.log('  → Patch 3: Wrap clonePointer(this) calls...');
  const bare = /(?<!toFfiPointer\()uniffiType([A-Za-z]*)ObjectFactory\.clonePointer\(this\)/g;
  const count = (content.match(bare) || []).length;
  if (count === 0) {
    if (content.includes('toFfiPointer(uniffiType')) {
      console.log('     Already wrapped (idempotent)');
      return content;
    }
    throw new Error(
      'Patch 3: no uniffiType*ObjectFactory.clonePointer(this) matches. uniffi-bindgen-node output may have changed.'
    );
  }
  const result = content.replace(
    bare,
    'toFfiPointer(uniffiType$1ObjectFactory.clonePointer(this))'
  );
  console.log(`     Wrapped ${count} occurrence(s)`);
  return result;
}

// Patch 4: Wrap .lower() calls for Name and Session objects
function patch4_WrapLowerCalls(content: string): string {
  console.log('  → Patch 4: Wrap .lower() calls for Name and Session...');

  const nameRegex = /(let [a-zA-Z_]*Arg = )FfiConverterTypeName\.lower\(([^)]+)\);/g;
  const nameCount = (content.match(nameRegex) || []).length;

  const sessionRegex = /(let [a-zA-Z_]*Arg = )FfiConverterTypeSession\.lower\(([^)]+)\);/g;
  const sessionCount = (content.match(sessionRegex) || []).length;

  const total = nameCount + sessionCount;
  if (total === 0) {
    if (
      content.includes('toFfiPointer(FfiConverterTypeName.lower') ||
      content.includes('toFfiPointer(FfiConverterTypeSession.lower')
    ) {
      console.log('     .lower() calls already wrapped (idempotent)');
      return content;
    }
    throw new Error(
      'Patch 4: no FfiConverterTypeName.lower / FfiConverterTypeSession.lower matches. uniffi-bindgen-node output may have changed.'
    );
  }

  let result = content;
  result = result.replace(nameRegex, '$1toFfiPointer(FfiConverterTypeName.lower($2));');
  result = result.replace(sessionRegex, '$1toFfiPointer(FfiConverterTypeSession.lower($2));');
  console.log(`     Wrapped ${total} occurrence(s) (Name: ${nameCount}, Session: ${sessionCount})`);
  return result;
}

// Patch 5: Fix clonePointer() and freePointer() methods
function patch5_FixPointerMethods(content: string): string {
  console.log('  → Patch 5: Fix clonePointer() and freePointer() methods...');

  if (
    content.includes('handleArgNum') &&
    /clonePointer\(obj: (?:App|CompletionHandle|Name|Service|Session)\)/.test(content) &&
    /freePointer\(handleArg: UnsafeMutableRawPointer\)/.test(content)
  ) {
    console.log('     Pointer methods already patched (idempotent)');
    return content;
  }

  const lines = content.split('\n');
  const output: string[] = [];
  let inCloneMethod = false;
  let inFreeMethod = false;
  let methodIndent = '';
  let cloneCount = 0;
  let freeCount = 0;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    if (/^\s+clonePointer\(obj: (?:App|CompletionHandle|Name|Service|Session)\): UnsafeMutableRawPointer \{/.test(line)) {
      inCloneMethod = true;
      methodIndent = line.match(/^(\s+)/)?.[1] || '';
      cloneCount++;
      output.push(line);
      continue;
    }

    if (/^\s+freePointer\(handleArg: UnsafeMutableRawPointer\): void \{/.test(line)) {
      inFreeMethod = true;
      methodIndent = line.match(/^(\s+)/)?.[1] || '';
      freeCount++;
      output.push(line);
      output.push(methodIndent + '  // Convert BigInt to Number for ffi-rs compatibility');
      output.push(methodIndent + '  const handleArgNum = typeof handleArg === \'bigint\' ? Number(handleArg) : handleArg;');
      continue;
    }

    if (inFreeMethod) {
      const modifiedLine = line.replace(/handleArg, callStatus/, 'handleArgNum, callStatus');
      output.push(modifiedLine);

      if (new RegExp(`^${methodIndent}\\},`).test(line)) {
        inFreeMethod = false;
      }
      continue;
    }

    if (inCloneMethod) {
      if (/^\s+const handleArg = this\.pointer\(obj\);$/.test(line)) {
        const indent = line.match(/^(\s+)/)?.[1] || '';
        output.push(line);
        output.push(indent + '// Convert BigInt to Number for ffi-rs compatibility');
        output.push(indent + 'const handleArgNum = typeof handleArg === \'bigint\' ? Number(handleArg) : handleArg;');
        continue;
      }

      if (/handleArg, callStatus/.test(line)) {
        output.push(line.replace(/handleArg, callStatus/, 'handleArgNum, callStatus'));
        continue;
      }

      if (/^\s+return uniffiCaller\.rustCall\(/.test(line)) {
        const callIndent = line.match(/^(\s+)/)?.[1] || '';

        const callBlock: string[] = [];
        callBlock.push(line.replace(/return /, 'const result = '));

        let j = i + 1;
        while (j < lines.length && !/^\s*\);$/.test(lines[j])) {
          callBlock.push(lines[j].replace(/handleArg, callStatus/, 'handleArgNum, callStatus'));
          j++;
        }

        if (j < lines.length) {
          callBlock.push(lines[j]);
        }

        output.push(...callBlock);
        output.push(callIndent + '// Convert result back to BigInt for React Native converters');
        output.push(callIndent + 'return typeof result === \'number\' ? BigInt(result) : result;');

        i = j;
        inCloneMethod = false;
        continue;
      }

      if (new RegExp(`^${methodIndent}\\},`).test(line)) {
        inCloneMethod = false;
      }
    }

    output.push(line);
  }

  if (cloneCount === 0 && freeCount === 0) {
    throw new Error(
      'Patch 5: no clonePointer/freePointer methods matched. uniffi-bindgen-node output may have changed.'
    );
  }
  console.log(`     Patched ${cloneCount} clonePointer and ${freeCount} freePointer method(s)`);
  return output.join('\n');
}

// Patch 6: Fix dataArg wrapping in publish methods
function patch6_FixDataArgWrapping(content: string): string {
  console.log('  → Patch 6: Fix RustBuffer wrapping for dataArg...');
  const regex = /let dataArg = FfiConverterArrayBuffer\.lower\(data\);/g;
  const count = (content.match(regex) || []).length;
  if (count === 0) {
    if (content.includes('UniffiRustBufferValue.allocateWithBytes(FfiConverterArrayBuffer.lower(data))')) {
      console.log('     dataArg already wrapped (idempotent)');
      return content;
    }
    throw new Error(
      'Patch 6: no "let dataArg = FfiConverterArrayBuffer.lower(data);" matches. uniffi-bindgen-node output may have changed.'
    );
  }
  const result = content.replace(
    regex,
    'let dataArg = UniffiRustBufferValue.allocateWithBytes(FfiConverterArrayBuffer.lower(data)).toStruct();'
  );
  console.log(`     Fixed ${count} occurrence(s)`);
  return result;
}

// Patch 7: Fix error extraction in uniffiCheckCallStatus
function patch7_FixErrorExtraction(content: string): string {
  console.log('  → Patch 7: Fix error extraction in uniffiCheckCallStatus...');

  if (content.includes('errorVariant.inner?.message')) {
    console.log('     Already patched (idempotent)');
    return content;
  }

  const legacyThrow = /throw new UniffiError\(enumName, errorVariant\);/;
  if (!legacyThrow.test(content)) {
    throw new Error(
      'Patch 7: expected throw new UniffiError(enumName, errorVariant); in liftError path. slim-bindings-sys.ts may have changed.'
    );
  }

  const lines = content.split('\n');
  const output: string[] = [];
  let inLiftError = false;
  let skipNextThrow = false;

  for (const line of lines) {
    if (/^\s+if \(liftError\) \{$/.test(line)) {
      inLiftError = true;
      output.push(line);
      continue;
    }

    if (inLiftError && /^\s+const \[enumName, errorVariant\] = liftError\(errorBufBytes\);$/.test(line)) {
      const indent = line.match(/^(\s+)/)?.[1] || '';
      output.push(line);
      output.push(indent + '// Extract variant name and message if errorVariant is an object with tag and inner');
      output.push(indent + 'if (errorVariant && typeof errorVariant === "object" && "tag" in errorVariant && "inner" in errorVariant) {');
      output.push(indent + '  const variantName = errorVariant.tag;');
      output.push(indent + '  const message = errorVariant.inner?.message || undefined;');
      output.push(indent + '  throw new UniffiError(enumName, variantName, message);');
      output.push(indent + '} else {');
      output.push(indent + '  // Fallback for simple string variant or unexpected format');
      output.push(indent + '  throw new UniffiError(enumName, String(errorVariant));');
      output.push(indent + '}');
      skipNextThrow = true;
      continue;
    }

    if (skipNextThrow && /^\s+throw new UniffiError\(enumName, errorVariant\);$/.test(line)) {
      skipNextThrow = false;
      inLiftError = false;
      continue;
    }

    output.push(line);
  }

  const joined = output.join('\n');
  if (!joined.includes('errorVariant.inner?.message')) {
    throw new Error('Patch 7: expected injected error handling block not found after transform.');
  }
  return joined;
}

// Main execution
try {
  if (fs.existsSync(BINDINGS_FILE)) {
    console.log(`\n📄 Patching ${path.basename(BINDINGS_FILE)}...`);
    let content = fs.readFileSync(BINDINGS_FILE, 'utf-8');

    content = patch1_FixConverterName(content);
    content = patch2_InjectHelper(content);
    content = patch3_WrapClonePointerThis(content);
    content = patch4_WrapLowerCalls(content);
    content = patch5_FixPointerMethods(content);
    content = patch6_FixDataArgWrapping(content);
    content = patchFlattenUniffiExportSymbols(content, 'slim-bindings-node.ts');

    fs.writeFileSync(BINDINGS_FILE, content, 'utf-8');
    console.log('✅ Patched slim-bindings-node.ts');
  } else {
    console.error(`❌ File not found: ${BINDINGS_FILE}`);
    process.exit(1);
  }

  if (fs.existsSync(SYS_FILE)) {
    console.log(`\n📄 Patching ${path.basename(SYS_FILE)}...`);
    let content = fs.readFileSync(SYS_FILE, 'utf-8');

    content = patchFlattenUniffiExportSymbols(content, 'slim-bindings-sys.ts');
    content = patch7_FixErrorExtraction(content);

    fs.writeFileSync(SYS_FILE, content, 'utf-8');
    console.log('✅ Patched slim-bindings-sys.ts');
  } else {
    console.error(`❌ File not found: ${SYS_FILE}`);
    process.exit(1);
  }

  console.log('\n✅ All patches applied successfully!');
} catch (error) {
  console.error('❌ Patching failed:', error);
  process.exit(1);
}
