import js from '@eslint/js'
import pluginVue from 'eslint-plugin-vue'
import tseslint from 'typescript-eslint'
import globals from 'globals'
import eslintConfigPrettier from 'eslint-config-prettier'

export default [
    js.configs.recommended,
    // Before Vue's, whose `.vue` parser must win; it hands `<script>` to `parserOptions.parser`.
    ...tseslint.configs.recommended.map((config) => ({...config, files: ['**/*.{ts,vue}']})),
    ...pluginVue.configs['flat/recommended'],
    eslintConfigPrettier,
    {
        files: ['**/*.{js,ts,vue}'],
        languageOptions: {ecmaVersion: 'latest', sourceType: 'module'},
        rules: {
            'vue/multi-word-component-names': 'off',
            'no-unused-vars': ['error', {argsIgnorePattern: '^_'}],
        },
    },
    // Only this config is JavaScript; tsc resolves names in the TypeScript.
    {
        files: ['**/*.js'],
        languageOptions: {globals: globals.node},
    },
    {
        files: ['**/*.{ts,vue}'],
        languageOptions: {
            parserOptions: {parser: tseslint.parser, extraFileExtensions: ['.vue']},
        },
        rules: {
            'no-undef': 'off',
            'no-unused-vars': 'off',
            '@typescript-eslint/no-unused-vars': ['error', {argsIgnorePattern: '^_', ignoreRestSiblings: true}],
            '@typescript-eslint/consistent-type-imports': 'error',
            '@typescript-eslint/no-explicit-any': 'error',
        },
    },
    {
        files: ['**/*.test.ts', 'src/test/**', '**/__tests__/**'],
        rules: {'@typescript-eslint/no-explicit-any': 'off'},
    },
    // Type-aware, production code only: a promise nobody handles is an unhandled rejection
    // (a fire-and-forget call says so with `void`), and a promise-returning function where a
    // void one is expected (an event handler, a watcher) is one nobody awaits either.
    {
        files: ['src/**/*.{ts,vue}'],
        ignores: ['src/**/*.test.ts', 'src/test/**', 'src/**/__tests__/**', 'src/testSetup.ts'],
        languageOptions: {
            parserOptions: {projectService: true, tsconfigRootDir: import.meta.dirname},
        },
        rules: {
            '@typescript-eslint/no-floating-promises': 'error',
            '@typescript-eslint/no-misused-promises': 'error',
            '@typescript-eslint/switch-exhaustiveness-check': ['error', {considerDefaultExhaustiveForUnions: true}],
        },
    },
    {
        ignores: ['dist/', 'coverage/', 'node_modules/'],
    },
]
