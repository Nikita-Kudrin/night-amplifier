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
        languageOptions: {
            ecmaVersion: 'latest',
            sourceType: 'module',
            // For the remaining JS: tsc resolves names in TypeScript.
            globals: {...globals.browser, ...globals.vitest, global: 'readonly'},
        },
        rules: {
            'vue/multi-word-component-names': 'off',
            'no-unused-vars': ['error', {argsIgnorePattern: '^_'}],
        },
    },
    {
        files: ['**/*.{ts,vue}'],
        languageOptions: {
            parserOptions: {parser: tseslint.parser, extraFileExtensions: ['.vue']},
        },
        rules: {
            'no-undef': 'off',
            'no-unused-vars': 'off',
            '@typescript-eslint/no-unused-vars': ['error', {argsIgnorePattern: '^_'}],
            '@typescript-eslint/consistent-type-imports': 'error',
            '@typescript-eslint/no-explicit-any': 'error',
        },
    },
    {
        files: ['**/*.test.{js,ts}', 'src/test/**', '**/__tests__/**'],
        rules: {'@typescript-eslint/no-explicit-any': 'off'},
    },
    {
        ignores: ['dist/', 'node_modules/'],
    },
]
