"use strict";

// Flat ESLint config. The pre-commit `eslint` hook lints committed `.js` files;
// the only first-party JavaScript in this repo is the npm launcher shim under
// `npm/`. Everything else (vendored deps, the TypeScript editor extension, the
// reference `air/` checkout, generated output) is ignored.

module.exports = [
  {
    ignores: [
      "**/node_modules/**",
      "editors/**",
      "air/**",
      "docs/**",
      "target/**",
      "style/**",
    ],
  },
  {
    files: ["npm/**/*.js"],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "commonjs",
      globals: {
        require: "readonly",
        module: "readonly",
        process: "readonly",
        console: "readonly",
        __dirname: "readonly",
        __filename: "readonly",
      },
    },
    rules: {
      "no-undef": "error",
      "no-unused-vars": "error",
    },
  },
];
