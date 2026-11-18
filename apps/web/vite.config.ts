import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In development the API runs separately: `cargo run -p mt-server -- --port 8080`.
export default defineConfig({
  plugins: [react()],
  server: { proxy: { "/api": "http://127.0.0.1:8080" } },
  build: { outDir: "dist", emptyOutDir: true, sourcemap: false },
});
