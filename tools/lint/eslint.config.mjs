// JavaScript lint for the SDK, the API functions, the web app and the scripts.
// Run: npm --prefix tools/lint ci && npm --prefix tools/lint run lint
import js from "@eslint/js";
import globals from "globals";

const root = new URL("../..", import.meta.url).pathname;

export default [
  {
    basePath: root,
    ignores: [
      "**/node_modules/**",
      "**/dist/**",
      "**/target/**",
      "**/out/**",
      "tempo/lib/**",
      "**/build/**",
      "**/*_js/**",
      "mirror-pool/**",
      "research_notes/**",
      "reports/**",
      "web/public/**",
    ],
  },
  {
    basePath: root,
    files: ["**/*.{js,mjs,jsx}"],
    ...js.configs.recommended,
    languageOptions: {
      ecmaVersion: "latest",
      sourceType: "module",
      globals: { ...globals.node, ...globals.browser },
      parserOptions: { ecmaFeatures: { jsx: true } },
    },
    rules: {
      ...js.configs.recommended.rules,
      "no-unused-vars": ["error", { argsIgnorePattern: "^_", caughtErrors: "none", ignoreRestSiblings: true }],
    },
  },
];
