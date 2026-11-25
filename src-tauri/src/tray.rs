//! 시스템 트레이 (P2-9 D7, 요구사항 §2.4): 창 닫기 -> 숨김(`closeToTray`), 메뉴 열기 / 작업 일시정지·재개 / 종료.
//! 종료는 `app.exit` -> `RunEvent::Exit` 에서 감시·작업 스레드를 멈춘다 (`lib.rs`).
//!
//! 트레이 메뉴 문구는 표시 언어(프론트 i18n)를 따른다 — 프론트가 번역한 문구를 command `set_tray_labels` 로 넘기면
//! [`labels`] 에 보관하고 설치된 메뉴 항목 텍스트를 즉시 갱신한다. 넘기기 전(시작 직후) 기본값은 영어.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use mam_core::ingest::Ingest;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Window, WindowEvent, Wry};

use crate::error::CommandError;
use crate::player_commands::PlayerService;
use crate::storage_commands::Storage;

mod labels;
pub use labels::TrayLabels;

/// 설치된 메뉴 항목 핸들 — 문구 갱신용
struct MenuHandles {
    open: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
    quit: MenuItem<Wry>,
    ingest: Arc<Ingest>,
}

static HANDLES: OnceLock<MenuHandles> = OnceLock::new();

/// 트레이가 실제로 만들어졌는가 — 없으면 창을 숨겨도 되살릴 방법이 없으므로 닫기를 숨김으로 바꾸지 않는다
static INSTALLED: AtomicBool = AtomicBool::new(false);

const MAIN_WINDOW: &str = "main";
const ID_OPEN: &str = "tray-open";
const ID_TOGGLE_JOBS: &str = "tray-toggle-jobs";
const ID_QUIT: &str = "tray-quit";

/// 숨겨진 메인 창을 다시 보여 앞으로 가져온다
pub fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        crate::player_commands::set_window_active(app, true);
    }
}

/// 숨기기 방식 - 트레이가 있어야 되살릴 수 있으므로 없으면 최소화로 대체한다
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum HideMode {
    Hide,
    Minimize,
}

fn hide_mode(tray_installed: bool) -> HideMode {
    if tray_installed { HideMode::Hide } else { HideMode::Minimize }
}

/// 숨기기 단축키: 메인 창을 트레이로 숨긴다(트레이가 없으면 최소화)
fn hide_window(app: &AppHandle) -> tauri::Result<()> {
    let Some(w) = app.get_webview_window(MAIN_WINDOW) else { return Ok(()) };
    // 보이지 않는 동안 워커 렌더를 멈춘다 (최소화·숨김은 JS 의 rAF 를 멈추지 않는다)
    crate::player_commands::set_window_active(app, false);
    match hide_mode(INSTALLED.load(Ordering::Relaxed)) {
        HideMode::Hide => w.hide(),
        HideMode::Minimize => w.minimize(),
    }
}

/// command `hide_main_window` - 재생 중인 플레이어를 사용자 일시정지로 멈춘 뒤 창을 숨긴다 (자동 재개 없음)
#[tauri::command]
pub async fn hide_main_window(app: AppHandle, svc: tauri::State<'_, Arc<PlayerService>>) -> Result<(), CommandError> {
    let svc = Arc::clone(&svc);
    // 일시정지 실패는 창 숨김을 막지 않는다
    if let Err(e) = tauri::async_runtime::spawn_blocking(move || svc.pause_for_hide()).await {
        tracing::error!(%e, "hide: pause task failed");
    }
    hide_window(&app).map_err(|e| CommandError::logged(crate::error::ErrorCode::Internal, "cannot hide the window", &e))
}

/// 종료 중 진행 중인 외부 AI 요청을 기다리는 동안 트레이 툴팁으로 알린다 (알림 풍선은 플러그인이 필요해 쓰지 않는다).
pub fn show_waiting_for_ai(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id("main-tray")
        && let Err(e) = tray.set_tooltip(Some(labels::current().ai_waiting))
    {
        tracing::warn!(%e, "tray: cannot set exit tooltip");
    }
}

/// command `set_tray_labels` - 번역된 트레이 문구를 보관하고 설치된 메뉴에 반영한다.
/// 문구는 장식이라 메뉴 갱신 실패는 로그만 남기고 성공으로 돌려준다.
#[tauri::command]
pub fn set_tray_labels(labels: TrayLabels) -> Result<(), CommandError> {
    let stored = labels::store(&labels);
    if let Some(h) = HANDLES.get() {
        let paused = h.ingest.jobs().paused_all();
        for (item, text) in [
            (&h.open, stored.open.as_str()),
            (&h.toggle, stored.toggle(paused)),
            (&h.quit, stored.quit.as_str()),
        ] {
            if let Err(e) = item.set_text(text) {
                tracing::warn!(%e, "tray: cannot update menu label");
            }
        }
    }
    Ok(())
}

/// 트레이 아이콘과 메뉴를 만든다 (`setup` 에서 호출).
pub fn install(app: &AppHandle, ingest: Arc<Ingest>) -> tauri::Result<()> {
    let current = labels::current();
    let open = MenuItem::with_id(app, ID_OPEN, &current.open, true, None::<&str>)?;
    let toggle = MenuItem::with_id(
        app,
        ID_TOGGLE_JOBS,
        current.toggle(ingest.jobs().paused_all()),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, ID_QUIT, &current.quit, true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open, &toggle, &PredefinedMenuItem::separator(app)?, &quit],
    )?;

    let mut builder = TrayIconBuilder::with_id("main-tray")
        .tooltip(labels::TOOLTIP)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            ID_OPEN => show_main_window(app),
            ID_TOGGLE_JOBS => {
                let Some(h) = HANDLES.get() else { return };
                // 화면(UI)에서 바꾼 상태와 어긋나지 않도록 실제 상태를 보고 뒤집는다
                let paused = h.ingest.jobs().paused_all();
                let result = if paused {
                    h.ingest.resume_jobs(None)
                } else {
                    h.ingest.pause_jobs(None)
                };
                match result {
                    Ok(_) => {
                        let _ = h.toggle.set_text(labels::current().toggle(!paused));
                    }
                    Err(e) => tracing::warn!(%e, "tray: cannot toggle jobs"),
                }
            }
            ID_QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    let _ = HANDLES.set(MenuHandles { open, toggle, quit, ingest });
    INSTALLED.store(true, Ordering::Relaxed);
    Ok(())
}

/// 메인 창의 닫기 요청: `closeToTray` 이면 닫지 않고 숨긴다. 설정을 읽지 못하면 평소처럼 닫는다.
pub fn on_window_event(window: &Window, event: &WindowEvent, storage: &Storage) {
    if window.label() != MAIN_WINDOW {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        let to_tray = storage
            .get_settings()
            .map(|s| s.settings.close_to_tray)
            .unwrap_or(false);
        if to_tray && INSTALLED.load(Ordering::Relaxed) {
            api.prevent_close();
            let _ = window.hide();
            crate::player_commands::set_window_active(window.app_handle(), false);
        }
    }
}

#[cfg(test)]
mod tests {
    // Categories: Business rule (문구 계약)
    use super::*;

    #[test]
    fn t_hidekey_win_01_tray_decides_hide_or_minimize() {
        assert_eq!(hide_mode(true), HideMode::Hide);
        assert_eq!(hide_mode(false), HideMode::Minimize);
    }

    #[test]
    fn t_prot_15_toggle_label_follows_the_paused_state() {
        let l = TrayLabels::default();
        assert_eq!(l.toggle(false), l.pause_jobs);
        assert_eq!(l.toggle(true), l.resume_jobs);
        assert_ne!(l.pause_jobs, l.resume_jobs);
    }
}
