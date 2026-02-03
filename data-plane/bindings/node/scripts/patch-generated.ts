#!/usr/bin/env tsx
// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/**
 * Post-generation patcher for Node.js bindings
 * 
 * Applies FFI compatibility fixes to generated code to bridge the gap between:
 * - uniffi-bindgen-node (generates React Native JSI types with BigInt pointers)
 * - ffi-rs (expects JavaScript Number for pointers)
 */

import * as fs from 'fs';
import * as path from 'path';

const GENERATED_DIR = process.argv[2] || path.join(__dirname, '../generated');
const BINDINGS_FILE = path.join(GENERATED_DIR, 'slim-bindings-node.ts');
const SYS_FILE = path.join(GENERATED_DIR, 'slim-bindings-sys.ts');

console.log('🔧 Applying FFI compatibility patches...');
console.log(`   Generated dir: ${GENERATED_DIR}`);

// Patch 1: FfiConverterBytes -> FfiConverterArrayBuffer
function patch1_FixConverterName(content: string): string {
  console.log('  → Patch 1: FfiConverterBytes references...');
  return content.replace(/FfiConverterBytes/g, 'FfiConverterArrayBuffer');
}

// Patch 2: Inject toFfiPointer helper function
function patch2_InjectHelper(content: string): string {
  console.log('  → Patch 2: Inject toFfiPointer helper...');
  const marker = "} from './slim-bindings-sys';";
  const helper = `

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
  
  // Only inject once
  if (content.includes('function toFfiPointer')) {
    return content;
  }
  
  return content.replace(marker, marker + helper);
}

// Patch 3: Wrap clonePointer(this) calls with toFfiPointer
function patch3_WrapClonePointerThis(content: string): string {
  console.log('  → Patch 3: Wrap clonePointer(this) calls (~74 occurrences)...');
  // Match: uniffiType*ObjectFactory.clonePointer(this)
  // Replace with: toFfiPointer(uniffiType*ObjectFactory.clonePointer(this))
  const regex = /uniffiType([A-Za-z]*)ObjectFactory\.clonePointer\(this\)/g;
  const count = (content.match(regex) || []).length;
  const result = content.replace(regex, 'toFfiPointer(uniffiType$1ObjectFactory.clonePointer(this))');
  console.log(`     Wrapped ${count} occurrences`);
  return result;
}

// Patch 4: Wrap .lower() calls for Name and Session objects
function patch4_WrapLowerCalls(content: string): string {
  console.log('  → Patch 4: Wrap .lower() calls (~35 occurrences)...');
  
  // Match: let *Arg = FfiConverterTypeName.lower(*);
  // Replace with: let *Arg = toFfiPointer(FfiConverterTypeName.lower(*));
  let result = content;
  let count = 0;
  
  // Pattern for Name and Session .lower() calls
  const nameRegex = /(let [a-zA-Z_]*Arg = )FfiConverterTypeName\.lower\(([^)]+)\);/g;
  count += (content.match(nameRegex) || []).length;
  result = result.replace(nameRegex, '$1toFfiPointer(FfiConverterTypeName.lower($2));');
  
  const sessionRegex = /(let [a-zA-Z_]*Arg = )FfiConverterTypeSession\.lower\(([^)]+)\);/g;
  count += (content.match(sessionRegex) || []).length;
  result = result.replace(sessionRegex, '$1toFfiPointer(FfiConverterTypeSession.lower($2));');
  
  console.log(`     Wrapped ${count} occurrences`);
  return result;
}

// Patch 5: Fix clonePointer() and freePointer() methods
function patch5_FixPointerMethods(content: string): string {
  console.log('  → Patch 5: Fix clonePointer() and freePointer() methods...');
  
  const lines = content.split('\n');
  const output: string[] = [];
  let inCloneMethod = false;
  let inFreeMethod = false;
  let methodIndent = '';
  let cloneCount = 0;
  let freeCount = 0;
  
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    
    // Detect clonePointer method start
    if (/^\s+clonePointer\(obj: (?:App|CompletionHandle|Name|Service|Session)\): UnsafeMutableRawPointer \{/.test(line)) {
      inCloneMethod = true;
      methodIndent = line.match(/^(\s+)/)?.[1] || '';
      cloneCount++;
      output.push(line);
      continue;
    }
    
    // Detect freePointer method start
    if (/^\s+freePointer\(handleArg: UnsafeMutableRawPointer\): void \{/.test(line)) {
      inFreeMethod = true;
      methodIndent = line.match(/^(\s+)/)?.[1] || '';
      freeCount++;
      output.push(line);
      // Add conversion right after method signature
      output.push(methodIndent + '  // Convert BigInt to Number for ffi-rs compatibility');
      output.push(methodIndent + '  const handleArgNum = typeof handleArg === \'bigint\' ? Number(handleArg) : handleArg;');
      continue;
    }
    
    // Inside freePointer - replace handleArg with handleArgNum
    if (inFreeMethod) {
      let modifiedLine = line.replace(/handleArg, callStatus/, 'handleArgNum, callStatus');
      output.push(modifiedLine);
      
      if (new RegExp(`^${methodIndent}\\},`).test(line)) {
        inFreeMethod = false;
      }
      continue;
    }
    
    // Inside clonePointer method
    if (inCloneMethod) {
      // Add conversion after const handleArg = this.pointer(obj);
      if (/^\s+const handleArg = this\.pointer\(obj\);$/.test(line)) {
        const indent = line.match(/^(\s+)/)?.[1] || '';
        output.push(line);
        output.push(indent + '// Convert BigInt to Number for ffi-rs compatibility');
        output.push(indent + 'const handleArgNum = typeof handleArg === \'bigint\' ? Number(handleArg) : handleArg;');
        continue;
      }
      
      // Replace handleArg with handleArgNum in FFI calls
      if (/handleArg, callStatus/.test(line)) {
        output.push(line.replace(/handleArg, callStatus/, 'handleArgNum, callStatus'));
        continue;
      }
      
      // Handle return statement - need to capture result and convert back to BigInt
      if (/^\s+return uniffiCaller\.rustCall\(/.test(line)) {
        const callIndent = line.match(/^(\s+)/)?.[1] || '';
        
        // Collect the entire rustCall block
        const callBlock: string[] = [];
        callBlock.push(line.replace(/return /, 'const result = '));
        
        let j = i + 1;
        while (j < lines.length && !/^\s*\);$/.test(lines[j])) {
          callBlock.push(lines[j].replace(/handleArg, callStatus/, 'handleArgNum, callStatus'));
          j++;
        }
        
        // Add the closing line
        if (j < lines.length) {
          callBlock.push(lines[j]);
        }
        
        // Output the modified call block
        output.push(...callBlock);
        output.push(callIndent + '// Convert result back to BigInt for React Native converters');
        output.push(callIndent + 'return typeof result === \'number\' ? BigInt(result) : result;');
        
        i = j; // Skip processed lines
        inCloneMethod = false;
        continue;
      }
      
      // Check for method end
      if (new RegExp(`^${methodIndent}\\},`).test(line)) {
        inCloneMethod = false;
      }
    }
    
    output.push(line);
  }
  
  console.log(`     Patched ${cloneCount} clonePointer and ${freeCount} freePointer methods`);
  return output.join('\n');
}

// Patch 6: Fix dataArg wrapping in publish methods
function patch6_FixDataArgWrapping(content: string): string {
  console.log('  → Patch 6: Fix RustBuffer wrapping for dataArg...');
  const regex = /let dataArg = FfiConverterArrayBuffer\.lower\(data\);/g;
  const count = (content.match(regex) || []).length;
  const result = content.replace(
    regex,
    'let dataArg = UniffiRustBufferValue.allocateWithBytes(FfiConverterArrayBuffer.lower(data)).toStruct();'
  );
  console.log(`     Fixed ${count} occurrences`);
  return result;
}

// Patch 7: Fix error extraction in uniffiCheckCallStatus
function patch7_FixErrorExtraction(content: string): string {
  console.log('  → Patch 7: Fix error extraction in uniffiCheckCallStatus...');
  
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
      // Skip this line - we already threw above
      skipNextThrow = false;
      inLiftError = false;
      continue;
    }
    
    output.push(line);
  }
  
  return output.join('\n');
}

// Main execution
try {
  // Patch slim-bindings-node.ts
  if (fs.existsSync(BINDINGS_FILE)) {
    console.log(`\n📄 Patching ${path.basename(BINDINGS_FILE)}...`);
    let content = fs.readFileSync(BINDINGS_FILE, 'utf-8');
    
    content = patch1_FixConverterName(content);
    content = patch2_InjectHelper(content);
    content = patch3_WrapClonePointerThis(content);
    content = patch4_WrapLowerCalls(content);
    content = patch5_FixPointerMethods(content);
    content = patch6_FixDataArgWrapping(content);
    
    fs.writeFileSync(BINDINGS_FILE, content, 'utf-8');
    console.log('✅ Patched slim-bindings-node.ts');
  } else {
    console.error(`❌ File not found: ${BINDINGS_FILE}`);
    process.exit(1);
  }
  
  // Patch slim-bindings-sys.ts
  if (fs.existsSync(SYS_FILE)) {
    console.log(`\n📄 Patching ${path.basename(SYS_FILE)}...`);
    let content = fs.readFileSync(SYS_FILE, 'utf-8');
    
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
