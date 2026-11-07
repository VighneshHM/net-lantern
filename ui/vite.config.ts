import { configDefaults, defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Dev loop: `netlantern --demo --no-browser --port 47800 --dev-origin http://localhost:5173`
// plus `npm run dev`. changeOrigin rewrites the Host header to the target so the
// server's Host check passes; the browser's Origin (localhost:5173) is allowed
// by --dev-origin.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    proxy: {
      "/api": {
        target: "http://127.0.0.1:47800",
        ws: true,
        changeOrigin: true,
      },
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    globals: false,
    exclude: [...configDefaults.exclude, "e2e/**"],
  },
});
