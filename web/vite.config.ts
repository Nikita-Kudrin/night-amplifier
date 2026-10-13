import {defineConfig} from 'vitest/config'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
    plugins: [vue()],
    server: {
        port: 8844,
        proxy: {
            '/api': {
                target: 'http://localhost:9955',
                changeOrigin: true
            },
            '/ws': {
                target: 'ws://localhost:9955',
                ws: true
            }
        }
    },
    build: {
        outDir: 'dist',
        assetsDir: 'assets'
    },
    test: {
        environment: 'happy-dom',
        globals: true,
        include: ['src/**/*.{test,spec}.{js,ts}'],
        setupFiles: ['src/testSetup.js'],
        // Puts back what `vi.stubGlobal` replaced (`fetch`'s unmocked-call guard among them).
        unstubGlobals: true,
        coverage: {
            provider: 'v8',
            reporter: ['text', 'html'],
            // Every source file counts, imported by a test or not; test code never does.
            include: ['src/**/*.{js,ts,vue}'],
            exclude: [
                'src/**/*.test.{js,ts}',
                'src/**/*.d.ts',
                'src/**/__tests__/**',
                'src/test/**',
                'src/testSetup.{js,ts}',
                'src/main.ts',
                'src/composables/api.types.{js,ts}',
                'src/composables/api.events.{js,ts}',
            ],
            thresholds: {
                lines: 80,
                branches: 80,
                functions: 80,
                statements: 80
            }
        }
    }
})
