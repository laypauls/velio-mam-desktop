/* 첫 페인트 전 테마 적용 (CSP 가 인라인 스크립트를 막아 별도 파일). 로직은 src/theme/resolve.ts 와 동일해야 한다 — theme-boot.test.ts 가 검증. */
(function () {
  var mode = "system", sd = "dim";
  try {
    var v = JSON.parse(window.localStorage.getItem("mam.theme"));
    if (v && (v.mode === "system" || v.mode === "light" || v.mode === "gray" || v.mode === "dim" || v.mode === "dark") && (v.systemDark === "dim" || v.systemDark === "dark")) {
      mode = v.mode;
      sd = v.systemDark;
    }
  } catch (e) {}
  var osDark = false;
  try {
    osDark = !!(window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches);
  } catch (e) {}
  var t = mode === "system" ? (osDark ? sd : "light") : mode;
  var root = window.document.documentElement;
  root.setAttribute("data-theme", t);
  if (t === "dim" || t === "dark") root.classList.add("dark");
  else root.classList.remove("dark");
})();
