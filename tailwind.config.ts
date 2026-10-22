import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./flowbar.html", "./src/**/*.{ts,tsx}"],
  theme: { extend: {} },
  plugins: [],
} satisfies Config;
