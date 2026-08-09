#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use asvc::{config::Config, daemon, i18n, paths::Paths};
use tauri::{
    Manager, WindowEvent,
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

fn initialize_locale(paths: &Paths) -> Result<(), String> {
    let config = Config::load(paths).map_err(|error| format!("{error:#}"))?;
    i18n::set_locale(config.locale);
    Ok(())
}

fn show_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    #[cfg(target_os = "macos")]
    let _ = app.set_dock_visibility(true);

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn run() -> i32 {
    let paths = Paths::discover();
    if std::env::args().nth(1).as_deref() == Some("__daemon") {
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("[asvc-daemon] {error:#}");
                return 1;
            }
        };
        return match runtime.block_on(daemon::run(paths)) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("[asvc-daemon] {error:#}");
                1
            }
        };
    }

    if let Err(error) = initialize_locale(&paths) {
        eprintln!("Desktop error: {error}");
        return 1;
    }

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main_window(app);
        }))
        .manage(commands::AppState::new(paths))
        .setup(|app| {
            let show_item = MenuItemBuilder::with_id("show", i18n::text("Show Asvc", "打开 Asvc"))
                .build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", i18n::text("Quit Asvc", "退出 Asvc"))
                .build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&show_item, &quit_item])
                .build()?;
            // Keep the status-bar glyph independent from the larger Dock/application icon;
            // the latter is embedded by the generated Tauri context at compile time.
            let tray_icon = Image::from_bytes(include_bytes!("../icons/asvc/tray@2x.png"))?;

            let tray = TrayIconBuilder::with_id("main-tray")
                .menu(&menu)
                .icon(tray_icon)
                .icon_as_template(false)
                .tooltip("Asvc")
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_main_window(app),
                    "quit" => app.exit(0),
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

            tray.build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                #[cfg(target_os = "macos")]
                let _ = window.app_handle().set_dock_visibility(false);
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::daemon_status,
            commands::set_locale,
            commands::get_services,
            commands::get_logs,
            commands::start_service,
            commands::stop_service,
            commands::restart_service,
            commands::start_all,
            commands::stop_all,
            commands::remove_service,
            commands::register_service,
        ]);

    match builder.run(tauri::generate_context!()) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Desktop error: {error:#}");
            1
        }
    }
}
