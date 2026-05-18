// Tailwind v4 uses CSS-first configuration via `@theme` in static/css/app.src.css.
// This file remains as a fallback content-scan hint for tooling that still
// expects a JS config (editors, some IDE plugins). Tokens live in the CSS source.
//
// If you switch back to v3, copy the @theme tokens into `theme.extend` here.

/** @type {import('tailwindcss').Config} */
export default {
  content: [
    './templates/**/*.html',
    './crates/**/*.rs',
  ],
};
