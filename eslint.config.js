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

// preserve literal data such as SVG paths, Markdown examples and regular expressions
const maxLineLength = {
  code: 120,
  tabWidth: 2,
  ignoreUrls: true,
  ignoreStrings: true,
  ignoreTemplateLiterals: true,
  ignoreRegExpLiterals: true
};

const baseRules = {
  '@stylistic/array-bracket-spacing': [ 'error', 'always', { objectsInArrays: false, arraysInArrays: false }],
  '@stylistic/brace-style': [ 'error', '1tbs', { allowSingleLine: false }],
  '@stylistic/comma-dangle': [ 'error', 'never' ],
  '@stylistic/computed-property-spacing': [ 'error', 'always' ],
  '@stylistic/eol-last': [ 'error', 'always' ],
  '@stylistic/indent': [ 'error', 2 ],
  '@stylistic/max-len': [ 'error', maxLineLength ],
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
  '@stylistic/no-multiple-empty-lines': [ 'error', { max: 1, maxBOF: 0, maxEOF: 0 }],
  '@stylistic/object-curly-spacing': [ 'error', 'always' ],
  '@stylistic/padding-line-between-statements': [
    'error',
    { blankLine: 'always', prev: [ 'const', 'let' ], next: '*' },
    { blankLine: 'any', prev: [ 'const', 'let' ], next: [ 'const', 'let' ] },
    { blankLine: 'always', prev: [ 'multiline-const', 'multiline-let' ], next: '*' },
    { blankLine: 'always', prev: '*', next: [ 'multiline-const', 'multiline-let' ] },
    { blankLine: 'always', prev: 'block-like', next: '*' },
    {
      blankLine: 'always',
      prev: '*',
      next: [ 'function', 'class', 'interface', 'type', 'return', 'if', 'for', 'while', 'switch', 'try' ]
    },
    { blankLine: 'always', prev: 'import', next: '*' },
    { blankLine: 'any', prev: 'import', next: 'import' }
  ],
  '@stylistic/quotes': [ 'error', 'single', { avoidEscape: true, allowTemplateLiterals: 'always' }],
  '@stylistic/semi': [ 'error', 'always' ],
  '@stylistic/space-in-parens': [ 'error', 'always', { exceptions: [ '{}', '[]', 'empty' ] }],
  '@stylistic/space-infix-ops': 'error',
  '@stylistic/template-curly-spacing': [ 'error', 'always' ],
  'curly': [ 'error', 'all' ],
  'capitalized-comments': [ 'error', 'never', { ignoreInlineComments: false }],
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
      '@stylistic/max-len': 'off',
      'vue/array-bracket-spacing': [ 'error', 'always', { objectsInArrays: false, arraysInArrays: false }],
      'vue/attributes-order': 'error',
      'vue/html-indent': [ 'error', 2 ],
      'vue/max-attributes-per-line': [ 'error', { singleline: 2, multiline: 1 }],
      'vue/max-len': [
        'error',
        { ...maxLineLength, template: 120, ignoreHTMLAttributeValues: true }
      ],
      // allow prose to wrap without adding whitespace around its rendered text
      'vue/multiline-html-element-content-newline': [
        'warn',
        {
          ignores: [
            'pre', 'textarea', 'p', 'a', 'abbr', 'audio', 'b', 'bdi', 'bdo', 'canvas', 'cite',
            'code', 'data', 'del', 'dfn', 'em', 'i', 'iframe', 'ins', 'kbd', 'label', 'map',
            'mark', 'noscript', 'object', 'output', 'picture', 'q', 'ruby', 's', 'samp', 'small',
            'span', 'strong', 'sub', 'sup', 'svg', 'time', 'u', 'var', 'video'
          ]
        }
      ],
      'vue/object-curly-spacing': [ 'error', 'always' ],
      'vue/space-in-parens': [ 'error', 'always', { exceptions: [ 'empty' ] }],
      'vue/space-infix-ops': 'error'
    }
  }
]);
