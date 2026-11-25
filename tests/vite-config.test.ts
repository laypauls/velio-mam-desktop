import { describe, expect, it } from "vitest";
import viteConfig from "../vite.config";

// Cases: Happy, Regression
// 회귀: Cargo 워크스페이스 출력(`target/`, 수십 GB)이 watch 대상이면 Vite(chokidar)가 시작 시 크롤링하며
// 이벤트 루프를 수 분간 막아 Tauri dev 창이 부트 스플래시에 머문다.
describe("vite.config server.watch.ignored", () => {
  it("T-ui-devcfg-01 Cargo 출력·src-tauri·temp 를 watch 에서 제외한다", () => {
    // Arrange
    const factory = viteConfig as unknown as () => {
      server: { watch: { ignored: string[] } };
    };

    // Act
    const ignored = factory().server.watch.ignored;

    // Assert
    expect(ignored).toEqual(
      expect.arrayContaining(["**/target/**", "**/src-tauri/**", "**/temp/**"]),
    );
  });
});
