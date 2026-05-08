#!/usr/bin/env tsx
// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/**
 * Packs a platform-specific npm package for @agntcy/slim-bindings (optional dependency).
 * Usage: npx tsx scripts/pack-platform.ts <RUST_TARGET> [VERSION]
 * Requires: task generate has been run for that TARGET (generated/ exists).
 * Output: dist/node-<platform>.tgz
 */

import * as fs from 'fs';
import * as path from 'path';
import { execSync } from 'child_process';
import { rustTargetToPlatformId } from './platform-id';

const TASKFILE_DIR = path.resolve(__dirname, '..');
const GENERATED_DIR = path.join(TASKFILE_DIR, 'generated');
const OUT_DIR = path.join(TASKFILE_DIR, '.platform-pkg');
const DIST_DIR = path.join(TASKFILE_DIR, 'dist');

/** Single shipped native artifact basename per Rust triple (matches Taskfile CANONICAL). */
function canonicalNativeLibraryBasename(rustTarget: string): string {
  if (rustTarget.includes('apple-darwin')) {
    return 'libslim_bindings.dylib';
  }
  if (rustTarget.includes('linux')) {
    return 'libslim_bindings.so';
  }
  if (rustTarget.includes('windows')) {
    return 'slim_bindings.dll';
  }
  throw new Error(`Cannot derive native library filename for target: ${rustTarget}`);
}

function main() {
  const rustTarget = process.argv[2];
  const version = process.argv[3] || readVersion();
  if (!rustTarget || !version) {
    console.error('Usage: pack-platform.ts <RUST_TARGET> [VERSION]');
    process.exit(1);
  }

  const platformId = rustTargetToPlatformId(rustTarget);
  const packageName = `@agntcy/slim-bindings-${platformId}`;

  if (!fs.existsSync(GENERATED_DIR)) {
    console.error('generated/ not found. Run: task generate TARGET=' + rustTarget);
    process.exit(1);
  }

  console.log(`Packing ${packageName}@${version} for ${rustTarget} (${platformId})...`);

  if (fs.existsSync(OUT_DIR)) {
    fs.rmSync(OUT_DIR, { recursive: true });
  }
  fs.mkdirSync(OUT_DIR, { recursive: true });
  fs.mkdirSync(DIST_DIR, { recursive: true });

  const tsconfig = {
    compilerOptions: {
      target: 'ES2020',
      module: 'ESNext',
      moduleResolution: 'node',
      lib: ['ES2020'],
      outDir: OUT_DIR,
      rootDir: GENERATED_DIR,
      declaration: true,
      declarationMap: false,
      sourceMap: false,
      skipLibCheck: true,
      strict: false,
      noImplicitAny: false,
      noEmitOnError: false,
      esModuleInterop: true,
      resolveJsonModule: true,
      types: ['node'],
    },
    include: [path.join(GENERATED_DIR, '**/*.ts')],
    exclude: ['node_modules'],
  };
  fs.writeFileSync(
    path.join(TASKFILE_DIR, 'tsconfig.pack-platform.json'),
    JSON.stringify(tsconfig, null, 2)
  );

  // Generated code is from uniffi-bindgen-node + patches; it can have type mismatches
  // (e.g. with uniffi-bindgen-react-native types). We only need JS + .d.ts for the pack.
  // noEmitOnError: false allows emit despite errors; tsc still exits non-zero, so we run
  // and then verify output exists instead of failing on exit code.
  const tscPath = path.join(TASKFILE_DIR, 'tsconfig.pack-platform.json');
  const quoted = JSON.stringify(tscPath);
  try {
    execSync(`npx -p typescript tsc -p ${quoted}`, {
      cwd: TASKFILE_DIR,
      encoding: 'utf-8',
      stdio: ['inherit', 'pipe', 'pipe'],
    });
  } catch (err: unknown) {
    // tsc exits non-zero when type errors exist even with noEmitOnError: false
    const e = err as { status?: number; stdout?: string; stderr?: string };
    const combined = `${e.stderr ?? ''}${e.stdout ?? ''}`.trim();
    console.warn(
      '[pack-platform] TypeScript reported diagnostics (emit may still succeed; review before release):\n' +
        (combined || '(no output captured)')
    );
  }
  const expectedJs = path.join(OUT_DIR, 'slim-bindings-node.js');
  const expectedDts = path.join(OUT_DIR, 'slim-bindings-node.d.ts');
  if (!fs.existsSync(expectedJs) || !fs.existsSync(expectedDts)) {
    console.error('tsc did not emit slim-bindings-node.js or .d.ts. Fix type errors or check compiler config.');
    process.exit(1);
  }

  // CI merges Linux-generated `generated/` (often includes libslim_bindings.so) with each
  // target's library copied as CANONICAL. Copying every *.so/*.dylib here shipped Linux ELF
  // inside @agntcy/slim-bindings-darwin-* tarballs. Only pack the library for this triple.
  const nativeBasename = canonicalNativeLibraryBasename(rustTarget);
  const nativeSrc = path.join(GENERATED_DIR, nativeBasename);
  if (!fs.existsSync(nativeSrc)) {
    console.error(
      `Missing ${nativeBasename} in ${GENERATED_DIR} for ${rustTarget}. ` +
        'Run task pack:platform:from-artifacts / generate for this target first.'
    );
    process.exit(1);
  }
  fs.copyFileSync(nativeSrc, path.join(OUT_DIR, nativeBasename));

  const generatedPkg = JSON.parse(
    fs.readFileSync(path.join(GENERATED_DIR, 'package.json'), 'utf-8')
  );
  const platformPkg = {
    name: packageName,
    version,
    description: `SLIM Node.js bindings (${platformId})`,
    main: 'slim-bindings-node.js',
    types: 'slim-bindings-node.d.ts',
    type: 'module',
    engines: { node: '>=18.0.0' },
    repository: {
      type: 'git',
      url: 'https://github.com/agntcy/slim.git',
      directory: 'data-plane/bindings/node',
    },
    license: 'Apache-2.0',
    dependencies: generatedPkg.dependencies || {},
    optionalDependencies: generatedPkg.optionalDependencies || {},
  };
  fs.writeFileSync(
    path.join(OUT_DIR, 'package.json'),
    JSON.stringify(platformPkg, null, 2)
  );

  execSync('npm install --omit=dev', { cwd: OUT_DIR, stdio: 'inherit' });

  const tgzName = `node-${platformId}.tgz`;
  execSync(`npm pack --pack-destination "${DIST_DIR}"`, {
    cwd: OUT_DIR,
    stdio: 'inherit',
  });

  const packed = fs.readdirSync(DIST_DIR).find((f) => f.endsWith('.tgz'));
  if (packed) {
    const dest = path.join(DIST_DIR, tgzName);
    if (path.resolve(path.join(DIST_DIR, packed)) !== path.resolve(dest)) {
      fs.renameSync(path.join(DIST_DIR, packed), dest);
    }
    console.log('Created:', dest);
  }

  fs.rmSync(path.join(TASKFILE_DIR, 'tsconfig.pack-platform.json'), { force: true });
}

function readVersion(): string {
  const pkgPath = path.join(TASKFILE_DIR, 'package.json');
  const pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf-8'));
  return pkg.version || '0.0.0';
}

main();
