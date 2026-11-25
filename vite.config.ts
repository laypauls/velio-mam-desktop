import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react(), tailwindcss()],

  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "./src"),
    },
  },

  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    passWithNoTests: true,
    // 화면 흐름 테스트(입력·Dialog 여러 단계)는 보통 2~4초지만 Rust 빌드가 겹쳐 CPU 가 포화되면 기본 5초를 넘는다.
    // 재시도 대신 실행 시간 상한만 넓힌다 (결정적 로직은 그대로) — 2026-10-29 CPU 100% 에서 간헐 timeout 확인
    testTimeout: 30_000,
    // after 훅을 정의 순서대로 실행한다 — setup.ts 의 cleanup(트리 언마운트)이 각 파일의 vi.unstubAllGlobals() 보다 먼저 돌아야
    // 늦게 도착한 비동기 렌더/effect 가 이미 제거된 전역(ResizeObserver 등)을 만나지 않는다 (기본 'stack' 은 반대 순서)
    sequence: { hooks: "list" },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri` (and `temp/`: dev library + MAMData written at runtime by `pnpm dev:temp`, and `target/`: Cargo build output, tens of GB)
      ignored: ["**/src-tauri/**", "**/temp/**", "**/target/**"],
    },
  },
}));
