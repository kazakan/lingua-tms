import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 개발 서버 설정: 포트 고정(tauri.conf.json 의 devUrl 과 같아야 함), Rust 쪽 파일은 감시하지 않음.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**", "**/engine/**"],
    },
  },
  build: {
    target: "es2022",
    // 데스크톱 앱이라 번들은 로컬 디스크에서 읽는다. 큰 청크 경고(500kB)는 의미가 없어 올려 둔다.
    chunkSizeWarningLimit: 3000,
  },
});
