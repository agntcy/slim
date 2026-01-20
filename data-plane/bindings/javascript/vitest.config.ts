import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    globals: true,
    environment: 'node',
    include: ['tests/**/*.test.ts'],
    testTimeout: 30000, // Integration tests may take longer
    hookTimeout: 30000,
    setupFiles: ['tests/setup.ts'],
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json', 'html'],
      include: ['generated/**/*.ts', 'tests/**/*.ts'],
      exclude: ['**/*.test.ts', '**/node_modules/**', 'tests/setup.ts']
    }
  }
})
