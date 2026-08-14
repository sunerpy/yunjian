use tauri::menu::MenuBuilder;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Emitter, Manager, Runtime, Window, WindowEvent};

const MAIN_WINDOW: &str = "main";
const NAVIGATE_EVENT: &str = "yunjian://navigate";
const TOGGLE_WINDOW: &str = "toggle-window";
const REVIEW_TODAY: &str = "review-today";
const SETTINGS: &str = "settings";
const QUIT: &str = "quit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayAction {
    ToggleWindow,
    ReviewToday,
    Settings,
    Quit,
}

fn menu_action(id: &str) -> Option<TrayAction> {
    match id {
        TOGGLE_WINDOW => Some(TrayAction::ToggleWindow),
        REVIEW_TODAY => Some(TrayAction::ReviewToday),
        SETTINGS => Some(TrayAction::Settings),
        QUIT => Some(TrayAction::Quit),
        _ => None,
    }
}

fn is_toggle_click(event: &TrayIconEvent) -> bool {
    matches!(
        event,
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Down,
            ..
        }
    )
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        window.show()?;
        window.set_focus()?;
    }
    Ok(())
}

fn toggle_main_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        if window.is_visible()? {
            window.hide()?;
        } else {
            window.show()?;
            window.set_focus()?;
        }
    }
    Ok(())
}

fn navigate<R: Runtime>(app: &AppHandle<R>, destination: &str) -> tauri::Result<()> {
    show_main_window(app)?;
    app.emit_to(MAIN_WINDOW, NAVIGATE_EVENT, destination)
}

fn handle_action<R: Runtime>(app: &AppHandle<R>, action: TrayAction) {
    let result = match action {
        TrayAction::ToggleWindow => toggle_main_window(app),
        TrayAction::ReviewToday => navigate(app, "recite"),
        TrayAction::Settings => navigate(app, "settings"),
        TrayAction::Quit => {
            app.exit(0);
            Ok(())
        }
    };
    if let Err(error) = result {
        tracing::warn!(?action, error = %error, "处理托盘操作失败");
    }
}

pub(crate) fn setup<R: Runtime>(app: &mut App<R>) -> tauri::Result<()> {
    let menu = MenuBuilder::new(app)
        .text(TOGGLE_WINDOW, "显示/隐藏主窗")
        .text(REVIEW_TODAY, "今日复习")
        .text(SETTINGS, "设置")
        .text(QUIT, "退出")
        .build()?;

    TrayIconBuilder::new()
        .icon(tauri::include_image!("./icons/tray.png"))
        .tooltip("云笺")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            if let Some(action) = menu_action(event.id().as_ref()) {
                handle_action(app, action);
            }
        })
        .on_tray_icon_event(|tray, event| {
            if is_toggle_click(&event) {
                handle_action(tray.app_handle(), TrayAction::ToggleWindow);
            }
        })
        .build(app)?;

    Ok(())
}

pub(crate) fn handle_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if window.label() != MAIN_WINDOW {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        if let Err(error) = window.hide() {
            tracing::warn!(error = %error, "隐藏主窗口失败");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_map_to_exactly_four_actions() {
        assert_eq!(menu_action(TOGGLE_WINDOW), Some(TrayAction::ToggleWindow));
        assert_eq!(menu_action(REVIEW_TODAY), Some(TrayAction::ReviewToday));
        assert_eq!(menu_action(SETTINGS), Some(TrayAction::Settings));
        assert_eq!(menu_action(QUIT), Some(TrayAction::Quit));
        assert_eq!(menu_action("unknown"), None);
    }

    #[test]
    fn only_left_button_down_toggles_the_window() {
        let click = |button, button_state| TrayIconEvent::Click {
            id: "test".into(),
            position: tauri::PhysicalPosition::default(),
            rect: tauri::Rect {
                position: tauri::Position::Physical(tauri::PhysicalPosition::default()),
                size: tauri::Size::Physical(tauri::PhysicalSize::default()),
            },
            button,
            button_state,
        };

        assert!(is_toggle_click(&click(
            MouseButton::Left,
            MouseButtonState::Down
        )));
        assert!(!is_toggle_click(&click(
            MouseButton::Left,
            MouseButtonState::Up
        )));
        assert!(!is_toggle_click(&click(
            MouseButton::Right,
            MouseButtonState::Down
        )));
    }
}
