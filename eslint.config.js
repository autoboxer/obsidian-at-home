import { defineConfig } from 'eslint/config';
import js from '@eslint/js';
import stylistic from '@stylistic/eslint-plugin';
import globals from 'globals';
import tseslint from 'typescript-eslint';
import vue from 'eslint-plugin-vue';
import vueParser from 'vue-eslint-parser';


const typescriptRules = Object.assign(
  {},
  ...tseslint.configs.recommended.map( ( config ) => config.rules ?? {})
);

const baseRules = {
  '@stylistic/array-bracket-spacing': [ 'error', 'always', { objectsInArrays: false, arraysInArrays: false }],
  '@stylistic/brace-style': [ 'error', '1tbs', { allowSingleLine: false }],
  '@stylistic/comma-dangle': [ 'error', 'never' ],
  '@stylistic/computed-property-spacing': [ 'error', 'always' ],
  '@stylistic/eol-last': [ 'error', 'always' ],
  '@stylistic/indent': [ 'error', 2 ],
  '@stylistic/no-multi-spaces': [
    'error',
    {
      ignoreEOLComments: true,
      exceptions: {
        Property: true,
        VariableDeclarator: true,
        ImportDeclaration: true
      }
    }
  ],
  '@stylistic/no-trailing-spaces': 'error',
  '@stylistic/object-curly-spacing': [ 'error', 'always' ],
  '@stylistic/quotes': [ 'error', 'single', { avoidEscape: true, allowTemplateLiterals: 'always' }],
  '@stylistic/semi': [ 'error', 'always' ],
  '@stylistic/space-in-parens': [ 'error', 'always', { exceptions: [ '{}', '[]', 'empty' ] }],
  '@stylistic/space-infix-ops': 'error',
  '@stylistic/template-curly-spacing': [ 'error', 'always' ],
  'curly': [ 'error', 'all' ],
  'no-control-regex': 'off',
  'no-console': 'warn'
};

const typescriptLintRules = {
  ...typescriptRules,
  ...baseRules,
  'no-unused-vars': 'off',
  '@typescript-eslint/no-unused-vars': [
    'error',
    {
      args: 'after-used',
      ignoreRestSiblings: true
    }
  ]
};


export default defineConfig([
  {
    ignores: [
      'node_modules/',
      'dist/',
      'build/',
      'src-tauri/',
      '.local-tests/',
      'coverage/',
      '.vite/'
    ]
  },
  {
    files: [ 'eslint.config.js' ],
    extends: [ js.configs.recommended ],
    languageOptions: {
      globals: globals.node
    },
    plugins: {
      '@stylistic': stylistic
    },
    rules: baseRules
  },
  {
    files: [ '**/*.ts' ],
    extends: [ js.configs.recommended, ...tseslint.configs.recommended ],
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node
      },
      parserOptions: {
        ecmaVersion: 'latest',
        sourceType: 'module'
      }
    },
    plugins: {
      '@stylistic': stylistic
    },
    rules: typescriptLintRules
  },
  {
    files: [ '**/*.vue' ],
    extends: [ js.configs.recommended, ...vue.configs[ 'flat/recommended' ] ],
    languageOptions: {
      globals: globals.browser,
      parser: vueParser,
      parserOptions: {
        ecmaVersion: 'latest',
        extraFileExtensions: [ '.vue' ],
        parser: tseslint.parser,
        sourceType: 'module'
      }
    },
    plugins: {
      '@stylistic': stylistic,
      '@typescript-eslint': tseslint.plugin
    },
    rules: {
      ...typescriptLintRules,
      'vue/array-bracket-spacing': [ 'error', 'always', { objectsInArrays: false, arraysInArrays: false }],
      'vue/attributes-order': 'error',
      'vue/html-indent': [ 'error', 2 ],
      'vue/max-attributes-per-line': [ 'error', { singleline: 2, multiline: 1 }],
      'vue/object-curly-spacing': [ 'error', 'always' ],
      'vue/space-in-parens': [ 'error', 'always', { exceptions: [ 'empty' ] }],
      'vue/space-infix-ops': 'error'
    }
  }
]);
